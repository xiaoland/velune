import Foundation

import VeluneBindings

/// Converts generated UniFFI records at the application boundary. The Swift UI
/// models remain platform-friendly; no JSON action envelope crosses this file.
enum BindingMapping {
    static func conversation(_ value: BindingConversationSummary) -> Conversation {
        Conversation(id: value.id, title: value.title, updatedAt: value.updatedAt,
                     runtimeID: value.runtimeId, cwd: value.cwd)
    }

    static func snapshot(_ value: BindingConversationSnapshot) -> ConversationSnapshot {
        ConversationSnapshot(revision: value.revision, conversation: conversation(value.conversation),
                             modelID: value.modelId, runState: runState(value.runState),
                             messages: value.messages.map(message), actions: actions(value.actions))
    }

    static func runState(_ value: BindingRunState) -> RunState {
        switch value { case .idle: return .idle; case .running: return .running; case .stopping: return .stopping; case .failed: return .failed }
    }

    static func actions(_ value: BindingConversationActions) -> ConversationActions {
        ConversationActions(canSend: value.canSend, canCancel: value.canCancel, canSwitch: value.canSwitch)
    }

    static func message(_ value: BindingMessage) -> Message {
        Message(id: value.id, role: value.role, blocks: value.blocks.map(block))
    }

    static func block(_ value: BindingMessageBlock) -> MessageBlock {
        switch value {
        case .text(let text): return MessageBlock(kind: "text", text: text)
        case .tool(let toolID, let title, let state): return MessageBlock(kind: "tool", toolID: toolID, title: title, state: state)
        case .notice(let text): return MessageBlock(kind: "notice", text: text)
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

    static func model(_ value: BindingModelDefinition) -> AIModel {
        AIModel(id: value.id, nickname: value.nickname, icon: value.icon,
                contextWindow: value.contextWindow, maxOutputTokens: value.maxOutputTokens,
                reasoningLevels: value.reasoningLevels)
    }

    static func protocolID(_ value: BindingGatewayProtocol) -> ProviderProtocol {
        switch value { case .chatCompletionsV1: return .chatCompletionsV1; case .responsesV1: return .responsesV1; case .messagesV1: return .messagesV1 }
    }

    static func providerBinding(_ value: BindingProviderModelBinding) -> ProviderModelBinding {
        ProviderModelBinding(modelID: value.modelId, externalModelID: value.externalModelId,
                             adapterMetadataJSON: value.adapterMetadataJson)
    }

    static func credentialSource(_ value: BindingCredentialSource) -> CredentialSource {
        CredentialSource(kind: "harness", harnessTypeID: value.harnessTypeId,
                         sourceInstanceID: value.sourceInstanceId, providerID: value.providerId,
                         settings: value.settings)
    }

    static func provider(_ value: BindingProviderDefinition) -> AIProvider {
        AIProvider(id: value.id, name: value.name, protocolID: protocolID(value.`protocol`),
                   endpoint: value.endpoint, credentialRef: value.credentialRef,
                   models: value.models.map(providerBinding),
                   credentialSource: value.credentialSource.map(credentialSource),
                   credentialGeneration: value.credentialGeneration)
    }

    static func gateway(_ value: BindingGatewayConfig) -> GatewayConfig {
        GatewayConfig(id: value.id, name: value.name, models: value.models.map(model),
                      providers: value.providers.map(provider),
                      routes: value.routes.map { ModelRoute(modelID: $0.modelId, providerID: $0.providerId) },
                      failover: FailoverPolicy(mode: .disabled))
    }

    static func bindingGateway(_ value: GatewayConfig) -> BindingGatewayConfig {
        BindingGatewayConfig(id: value.id, name: value.name, models: value.models.map { BindingModelDefinition(id: $0.id, nickname: $0.nickname, icon: $0.icon, maxOutputTokens: $0.maxOutputTokens, contextWindow: $0.contextWindow, reasoningLevels: $0.reasoningLevels) }, providers: value.providers.map { BindingProviderDefinition(id: $0.id, name: $0.name, protocol: bindingProtocol($0.protocolID), endpoint: $0.endpoint, credentialRef: $0.credentialRef, credentialSource: $0.credentialSource.map(bindingCredentialSource), credentialGeneration: $0.credentialGeneration ?? 0, models: $0.models.map(bindingProviderModel) ) }, routes: value.routes.map { BindingRoute(modelId: $0.modelID, providerId: $0.providerID) }, failover: BindingFailoverPolicy(mode: .disabled))
    }

    private static func bindingProtocol(_ value: ProviderProtocol) -> BindingGatewayProtocol {
        switch value { case .chatCompletionsV1: return .chatCompletionsV1; case .responsesV1: return .responsesV1; case .messagesV1: return .messagesV1 }
    }

    private static func bindingProviderModel(_ value: ProviderModelBinding) -> BindingProviderModelBinding {
        BindingProviderModelBinding(modelId: value.modelID, externalModelId: value.externalModelID, adapterMetadataJson: value.adapterMetadataJSON)
    }

    private static func bindingCredentialSource(_ value: CredentialSource) -> BindingCredentialSource {
        BindingCredentialSource(kind: .harness, harnessTypeId: value.harnessTypeID, sourceInstanceId: value.sourceInstanceID, providerId: value.providerID, settings: value.settings)
    }

    static func bindingRuntime(_ value: RuntimeInstance) -> BindingRuntimeInstance {
        BindingRuntimeInstance(id: value.id, name: value.name, typeId: value.typeID, gatewayId: value.gatewayID, settings: value.settings, modelId: value.modelID)
    }

    static func bindingSelection(_ value: ProviderImportSelection) -> BindingImportSelection {
        BindingImportSelection(providerId: value.providerId, modelIds: value.modelIds, modelMappings: value.modelMappings)
    }

    static func bindingSource(_ value: CredentialSource) -> BindingCredentialSource {
        bindingCredentialSource(value)
    }

    static func runtime(_ value: BindingRuntimeInstance) -> RuntimeInstance {
        RuntimeInstance(id: value.id, name: value.name, typeID: value.typeId,
                        gatewayID: value.gatewayId, settings: value.settings, modelID: value.modelId)
    }

    static func runtimeType(_ value: BindingRuntimeTypeDescriptor) -> RuntimeTypeDescriptor {
        RuntimeTypeDescriptor(id: value.id, name: value.name, fields: value.fields.map(settingField), actions: value.actions.map(settingAction))
    }

    static func configuration(_ value: BindingConfigurationSnapshot) -> (conversations: [Conversation], connections: [Connection], models: [AIModel], gateways: [GatewayConfig], runtimes: [RuntimeInstance], runtimeTypes: [RuntimeTypeDescriptor], credentialTypes: [RuntimeTypeDescriptor], importTypes: [RuntimeTypeDescriptor], protocols: [ProtocolDescriptor], activeRuntimeID: String?) {
        (value.conversations.map(conversation), value.connections.map { Connection(id: $0.id, name: $0.name, state: $0.state, capabilities: $0.capabilities) }, value.models.map(model), value.gateways.map(gateway), value.runtimeInstances.map(runtime), value.runtimeTypes.map(runtimeType), value.credentialSourceTypes.map(runtimeType), value.providerImportTypes.map(runtimeType), value.protocols.map { ProtocolDescriptor(id: protocolID($0.id), name: $0.name, supported: $0.supported) }, value.activeRuntimeInstanceId)
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
        ProviderImportCandidate(id: value.id, name: value.name, protocol: value.`protocol`, endpoint: value.endpoint ?? "", canImport: value.canImport, alreadyImported: value.alreadyImported, credentialStatus: value.credentialStatus, issues: value.issues, models: value.models.map { ProviderImportModel(id: $0.id, externalModelId: $0.externalModelId, name: $0.name, contextWindow: $0.contextWindow, maxOutputTokens: $0.maxOutputTokens, reasoningLevels: $0.reasoningLevels, canImport: $0.canImport, issues: $0.issues) })
    }

    static func authenticationMetadata(_ value: BindingAuthenticationMetadata) -> AuthenticationMetadata {
        AuthenticationMetadata(configured: value.configured, capabilities: AuthenticationCapabilities(protocol: protocolID(value.capabilities.`protocol`), endpoint: value.capabilities.endpoint, explicitOutputCap: value.capabilities.explicitOutputCap, temperature: value.capabilities.temperature), actions: value.actions?.map(settingAction))
    }

    static func authenticationProgress(_ value: BindingAuthenticationProgress) -> AuthenticationData {
        AuthenticationData(running: value.running, events: value.events.map { AuthenticationEvent(type: $0.typeId, id: $0.id, prompt: $0.prompt.map { AuthenticationPromptBody(kind: $0.kind, text: $0.text, options: $0.options?.map { SettingOption(id: $0.id, label: $0.label) }) }, notification: $0.notification.map { AuthenticationNotification(kind: $0.kind, id: $0.id, text: $0.text, url: $0.url, instructions: $0.instructions, userCode: $0.userCode, verificationUri: $0.verificationUri) }, ok: $0.ok, error: $0.error, cancelled: $0.cancelled) }, gateways: value.gateways?.map(gateway), requiresReconnect: value.requiresReconnect)
    }
}
