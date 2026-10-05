import Foundation

struct Conversation: Codable, Sendable, Identifiable, Equatable {
    let id: String
    var title: String
    var updatedAt: String?
    var runtimeID: String
    var cwd: String?
    enum CodingKeys: String, CodingKey { case id, title, updatedAt, cwd; case runtimeID = "runtimeId" }
}

enum RunState: String, Codable {
    case idle
    case running
    case stopping
    case failed
}

struct ConversationActions: Codable, Sendable, Equatable {
    var canSend: Bool
    var canCancel: Bool
    var canSwitch: Bool
}

struct Message: Codable, Sendable, Identifiable, Equatable {
    let id: String
    var role: String
    var blocks: [MessageBlock]

    var text: String {
        blocks.compactMap { block in
            block.kind == "text" || block.kind == "notice" ? block.text : nil
        }.joined()
    }
}

struct MessageBlock: Codable, Sendable, Equatable {
    var kind: String
    var text: String?
    var toolID: String?
    var title: String?
    var state: String?

    init(kind: String, text: String? = nil, toolID: String? = nil, title: String? = nil, state: String? = nil) {
        self.kind = kind
        self.text = text
        self.toolID = toolID
        self.title = title
        self.state = state
    }

    static func text(_ value: String) -> MessageBlock {
        MessageBlock(kind: "text", text: value)
    }

    static func notice(_ value: String) -> MessageBlock {
        MessageBlock(kind: "notice", text: value)
    }
}

struct ConversationSnapshot: Codable, Sendable, Equatable {
    var revision: UInt64
    var conversation: Conversation
    var modelID: String?
    var runState: RunState
    var messages: [Message]
    var actions: ConversationActions
    enum CodingKeys: String, CodingKey { case revision, conversation, runState, messages, actions; case modelID = "modelId" }
}

struct Connection: Codable, Sendable, Identifiable, Equatable {
    let id: String
    var name: String
    var state: String
    var capabilities: [String]
}

struct AIModel: Codable, Sendable, Identifiable, Equatable {
    var id: String
    var nickname: String
    var icon: String?
    var contextWindow: UInt32? = nil
    var maxOutputTokens: UInt32
    var reasoningLevels: [String]
}

// Adapter-owned metadata stays opaque to the platform UI and is validated by
// the adapter at the ABI boundary.
struct ProviderModelBinding: Codable, Sendable, Equatable {
    var modelID: String
    var externalModelID: String
    var adapterMetadataJSON: String?
    enum CodingKeys: String, CodingKey { case modelID = "modelId", externalModelID = "externalModelId", adapterMetadataJSON = "adapterMetadataJson" }
}

enum ProviderProtocol: String, Codable, Sendable, CaseIterable, Identifiable {
    case chatCompletionsV1
    case responsesV1
    case messagesV1
    var id: String { rawValue }
}

struct CredentialSource: Codable, Sendable, Equatable {
    var kind: String = "harness"
    var harnessTypeID: String
    var sourceInstanceID: String?
    var providerID: String
    var settings: [String: String]
    enum CodingKeys: String, CodingKey {
        case kind, settings
        case harnessTypeID = "harnessTypeId"
        case sourceInstanceID = "sourceInstanceId"
        case providerID = "providerId"
    }
}

struct CredentialSourceType: Codable, Sendable, Identifiable, Equatable {
    var id: String
    var name: String
    var fields: [SettingField]
    var actions: [SettingAction]
}

struct AIProvider: Codable, Sendable, Identifiable, Equatable {
    var id: String
    var name: String
    var protocolID: ProviderProtocol
    var endpoint: String
    var credentialRef: String?
    var models: [ProviderModelBinding]
    var credentialSource: CredentialSource? = nil
    var credentialGeneration: UInt64? = nil
    enum CodingKeys: String, CodingKey { case id, name, endpoint, credentialRef, credentialSource, credentialGeneration, models; case protocolID = "protocol" }
}

struct ModelRoute: Codable, Sendable, Identifiable, Equatable {
    var id: String { modelID }
    var modelID: String
    var providerID: String
    enum CodingKeys: String, CodingKey { case modelID = "modelId"; case providerID = "providerId" }
}

enum FailoverMode: String, Codable, Sendable { case disabled }
struct FailoverPolicy: Codable, Sendable, Equatable { var mode: FailoverMode = .disabled }
struct GatewayConfig: Codable, Sendable, Equatable {
    var id: String = "default"
    var name: String = "默认网关"
    var models: [AIModel] = []
    var providers: [AIProvider] = []
    var routes: [ModelRoute] = []
    var failover: FailoverPolicy = FailoverPolicy()
}

struct RuntimeInstance: Codable, Sendable, Identifiable, Equatable {
    var id: String
    var name: String
    var typeID: String
    var gatewayID: String
    var settings: [String: String]
    var modelID: String?
    enum CodingKeys: String, CodingKey { case id, name, settings; case typeID = "typeId"; case gatewayID = "gatewayId"; case modelID = "modelId" }
}

struct RuntimeTypeDescriptor: Codable, Sendable, Identifiable, Equatable {
    var id: String
    var name: String
    var fields: [SettingField]
    var actions: [SettingAction]
}

struct ProtocolDescriptor: Codable, Sendable, Identifiable, Equatable {
    var id: ProviderProtocol
    var name: String
    var supported: Bool
}

enum SettingKind: String, Codable {
    case text
    case filePath
    case directoryPath
    case choice
}

struct SettingOption: Codable, Sendable, Identifiable, Equatable {
    let id: String
    var label: String
}

struct SettingAction: Codable, Sendable, Identifiable, Equatable {
    let id: String
    var label: String
}

struct ExecutableDiscoverySpec: Codable, Sendable, Equatable {
    var command: String
    var minimumVersion: String
}

struct SettingField: Codable, Sendable, Identifiable, Equatable {
    var id: String { key }
    let key: String
    var label: String
    var kind: SettingKind
    var required: Bool
    var value: String
    var options: [SettingOption]
    var help: String?
    var executableDiscovery: ExecutableDiscoverySpec? = nil
}

struct AuthenticationPrompt: Decodable, Sendable, Identifiable {
    var id: String
    var kind: String
    var text: String
    var options: [SettingOption]?
}
struct AuthenticationNotification: Decodable, Sendable {
    var kind: String
    var id: String?
    var text: String?
    var url: String?
    var instructions: String?
    var userCode: String?
    var verificationUri: String?
}
struct AuthenticationEvent: Decodable, Sendable {
    var type: String
    var id: String?
    var prompt: AuthenticationPromptBody?
    var notification: AuthenticationNotification?
    var ok: Bool?
    var error: String?
    var cancelled: Bool?
}
struct AuthenticationPromptBody: Decodable, Sendable {
    var kind: String
    var text: String
    var options: [SettingOption]?
}
struct AuthenticationData: Decodable, Sendable {
    var running: Bool
    var events: [AuthenticationEvent]
    var gateways: [GatewayConfig]?
    var requiresReconnect: Bool?
}

struct AuthenticationMetadata: Decodable, Sendable {
    var configured: Bool
    var capabilities: AuthenticationCapabilities
    var actions: [SettingAction]?
}
struct AuthenticationCapabilities: Decodable, Sendable {
    var `protocol`: ProviderProtocol
    var endpoint: String
    var explicitOutputCap: Bool
    var temperature: Bool
}
struct AuthenticationInspection: Decodable, Sendable { var metadata: AuthenticationMetadata }
