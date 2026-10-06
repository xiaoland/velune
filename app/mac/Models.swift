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
    var modelRecordKey: String?
    var runState: RunState
    var messages: [Message]
    var pendingInteractions: [RuntimeInteraction] = []
    var actions: ConversationActions
    enum CodingKeys: String, CodingKey { case revision, conversation, runState, messages, pendingInteractions, actions; case modelRecordKey = "modelRecordKey" }
}

struct Connection: Codable, Sendable, Identifiable, Equatable {
    let id: String
    var name: String
    var state: String
    var capabilities: [String]
}

struct ProviderModel: Codable, Sendable, Identifiable, Equatable {
    var id: String { recordKey }
    var recordKey: String
    var providerModelID: String
    var nickname: String = ""
    var icon: String? = nil
    var contextWindow: UInt32? = nil
    var maxOutputTokens: UInt32? = nil
    var reasoningLevels: [String]? = nil
    var adapterMetadataJSON: String? = nil
    var displayName: String { nickname.isEmpty ? providerModelID : nickname }
}
struct ModelChoice: Identifiable, Sendable {
    var id: String { recordKey }
    var recordKey: String
    var displayName: String
    var protocolID: ProviderProtocol
}
struct ModelTemplate: Codable, Sendable, Identifiable, Equatable {
    var id: String = ""
    var name: String
    var suggestedProviderModelID: String
    var nickname: String = ""
    var icon: String? = nil
    var contextWindow: UInt32? = nil
    var maxOutputTokens: UInt32? = nil
    var reasoningLevels: [String]? = nil
    var model: ProviderModel { ProviderModel(recordKey: "", providerModelID: suggestedProviderModelID, nickname: nickname, icon: icon, contextWindow: contextWindow, maxOutputTokens: maxOutputTokens, reasoningLevels: reasoningLevels) }
}
enum ProviderProtocol: String, Codable, Sendable, CaseIterable, Identifiable {
    case chatCompletionsV1
    case responsesV1
    case messagesV1
    var id: String { rawValue }
}

enum AuthenticationMethod: String, Codable, Sendable {
    case apiKey, oauth, unconfigured
    var label: String {
        switch self { case .apiKey: return "API key"; case .oauth: return "OAuth"; case .unconfigured: return "尚未配置" }
    }
}

struct ProviderAuthentication: Codable, Sendable, Equatable {
    var method: AuthenticationMethod = .unconfigured
    var configured: Bool = false
    var provenance: String? = nil
    var actions: [SettingAction] = []
}
enum AuthenticationEdit: Sendable { case keep, setAPIKey(String), clear }
struct AIProvider: Codable, Sendable, Identifiable, Equatable {
    var id: String
    var name: String
    var protocolID: ProviderProtocol
    var endpoint: String
    var authentication: ProviderAuthentication = ProviderAuthentication()
    var models: [ProviderModel]
}

enum FailoverMode: String, Codable, Sendable { case disabled }
struct FailoverPolicy: Codable, Sendable, Equatable { var mode: FailoverMode = .disabled }
struct GatewayConfig: Codable, Sendable, Equatable {
    var id: String = "default"
    var name: String = "默认网关"
    var providers: [AIProvider] = []
    var failover: FailoverPolicy = FailoverPolicy()
}

struct RuntimeInstance: Codable, Sendable, Identifiable, Equatable {
    var id: String
    var name: String
    var typeID: String
    var gatewayID: String
    var settings: [String: String]
    var modelRecordKey: String?
    enum CodingKeys: String, CodingKey { case id, name, settings; case typeID = "typeId"; case gatewayID = "gatewayId"; case modelRecordKey = "modelRecordKey" }
}

struct RuntimeTypeDescriptor: Codable, Sendable, Identifiable, Equatable {
    var id: String
    var familyID: String
    var versionRegex: String
    var supportedProtocols: [ProviderProtocol]
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

struct RuntimeInteraction: Codable, Sendable, Identifiable, Equatable {var id:String;var kind:InteractionKind}
enum InteractionKind: Codable, Sendable, Equatable {
    case approval(title:String,detail:String,options:[InteractionOption])
    case userInput(questions:[InteractionQuestion])
}
struct InteractionOption: Codable, Sendable, Identifiable, Equatable {var id:String;var label:String}
struct InteractionQuestion: Codable, Sendable, Identifiable, Equatable {var id:String;var text:String;var options:[InteractionOption];var secret:Bool}
enum RuntimeInteractionReply: Sendable {case decision(String), answers([InteractionAnswer]), cancel}
struct InteractionAnswer: Sendable {var questionID:String;var values:[String]}

struct CatalogModel: Identifiable, Sendable {
    var sourceProviderID: String
    var sourceProviderName: String
    var modelID: String
    var name: String
    var contextWindow: UInt32?
    var maxOutputTokens: UInt32?
    var reasoningLevels: [String]?
    var id: String { "\(sourceProviderID.count):\(sourceProviderID)\(modelID)" }
    var template: ModelTemplate {
        ModelTemplate(name: "\(name) · \(sourceProviderName) (models.dev)", suggestedProviderModelID: modelID, nickname: name, contextWindow: contextWindow, maxOutputTokens: maxOutputTokens, reasoningLevels: reasoningLevels)
    }
}
