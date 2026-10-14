import Foundation

struct Conversation: Codable, Sendable, Identifiable, Equatable {
    let id: String
    var title: String
    var updatedAtUnixMs: Int64?
    var createdAtUnixMs: Int64? = nil
    var runtimeID: String
    var cwd: String?
    var canRename = true
    var canDelete = true
    /// A native sub-agent's conversation: opened from its parent's transcript,
    /// not listed in the sidebar, and continued only by the parent's agent.
    var parentConversationID: String? = nil
    enum CodingKeys: String, CodingKey { case id, title, updatedAtUnixMs, createdAtUnixMs, cwd, canRename, canDelete; case runtimeID = "runtimeId"; case parentConversationID = "parentConversationId" }
}

enum RunState: String, Codable {
    case idle
    case running
    case stopping
    case failed
    /// Best-effort: the native records show a turn running outside Velune.
    case elsewhere
}

struct ConversationActions: Codable, Sendable, Equatable {
    var canSend: Bool
    var canCancel: Bool
    var canSwitch: Bool
}

enum MessageRole: String, Codable, Sendable { case user, assistant, tool, system }
enum ToolState: String, Codable, Sendable { case pending, running, completed, failed }

struct Message: Codable, Sendable, Identifiable, Equatable {
    let id: String
    var role: MessageRole
    var completed: Bool = true
    var timestampUnixMs: Int64? = nil
    var blocks: [MessageBlock]
    /// Core delivery sequence of this content; a changed message gets a new stamp.
    var stamp: UInt64 = 0
    /// Visible message text; reasoning and tool output are separate disclosures.
    var text: String { blocks.compactMap { block in switch block { case .text(let text), .notice(let text): return text; case .reasoning, .tool, .image, .file, .agentSender: return nil } }.joined() }
    var hasDeferredPayload: Bool {
        blocks.contains { block in
            switch block {
            case .reasoning(_, .deferred), .tool(_, _, _, .deferred?, _), .image(_, .data(.deferred)): return true
            case .text, .notice, .reasoning(_, .inline), .tool, .image, .file, .agentSender: return false
            }
        }
    }
    /// An image whose content is still to be read for its thumbnail.
    var hasDeferredImage: Bool { blocks.contains { if case .image(_, .data(.deferred)) = $0 { return true }; return false } }
}

enum MessageBlock: Codable, Sendable, Equatable {
    case text(String)
    /// Reasoning shown by its summary; the payload is the text under it.
    case reasoning(summary: String, text: MessagePayload)
    /// `conversation` is the conversation a Velune collaboration tool started or addressed.
    case tool(id: String?, title: String, state: ToolState, output: MessagePayload?, conversation: String?)
    case notice(String)
    /// An image the user attached.
    case image(mediaType: String?, source: ImageSource)
    /// A non-image file the user attached by absolute path.
    case file(path: String)
    /// The conversation whose agent sent this user-side message through Velune.
    case agentSender(conversationID: String, runtimeName: String, title: String)
}

enum ImageSource: Codable, Sendable, Equatable {
    case path(String)
    /// Base64 content.
    case data(MessagePayload)
}

/// Large reasoning and tool output arrive by size and are read when shown.
enum MessagePayload: Codable, Sendable, Equatable {
    case inline(String)
    case deferred(bytes: UInt64)
}

/// Position in the core's delivery history of one conversation view.
struct TranscriptCursor: Codable, Sendable, Equatable {
    var generation: UInt64
    var sequence: UInt64
}

struct MessageIdentityConfirmation: Codable, Sendable, Equatable {
    var previousID: String
    var currentID: String
}

struct TranscriptTurn: Codable, Sendable, Equatable, Identifiable {
    var id: String
    var userMessageID: String
    var workMessageIDs: [String]
    /// The closing assistant message that carries the reply; harness lifecycle determines whether work is running.
    var lastMessageID: String?
    /// Blocks of the last message before this index are work; the rest is the reply.
    var replyStartBlock: Int = 0
    var durationMs: UInt64?
    var isRunning: Bool
    var settings: TurnSettings? = nil
    var usage: TurnUsage? = nil
}
/// Tokens a turn used, as the runtime recorded them: input includes cache
/// reads and writes, output includes reasoning; `nil` where it records none.
/// The rest is where the conversation stood when the turn ended.
struct TurnUsage: Codable, Sendable, Equatable {
    var inputTokens: UInt64?
    var cacheReadTokens: UInt64?
    var cacheWriteTokens: UInt64?
    var outputTokens: UInt64?
    var reasoningTokens: UInt64?
    /// What the turn's last request left in the context.
    var contextTokens: UInt64? = nil
    var contextWindow: UInt64? = nil
    /// The harness's own estimate for the whole conversation so far.
    var sessionCostMicroUSD: UInt64? = nil
    /// Account limits the harness last reported.
    var rateLimits: [RateLimit] = []
}
struct RateLimit: Codable, Sendable, Equatable {
    var windowMinutes: UInt64?
    var usedPercent: UInt32
    var resetsAtUnixMs: Int64?
}

/// What a turn ran with, as far as the runtime's record and Velune's own
/// request say; each part is absent when nothing says.
struct TurnSettings: Codable, Sendable, Equatable {
    var runtimeID: String?
    /// Display text.
    var model: String?
    /// In the runtime's vocabulary.
    var effort: String?
    /// An ID the runtime type declares, or the record's own value.
    var permissionMode: String?
}

enum TranscriptItem: Codable, Sendable, Equatable {
    case message(id: String, messageID: String)
    case work(turnID: String)
}
struct TranscriptOutlineEntry: Codable, Sendable, Equatable, Identifiable {
    var id: String
    var messageID: String
}
struct TranscriptMessageIdentity: Codable, Sendable, Equatable {
    var id: String
    var messageID: String
}

struct ConversationSnapshot: Codable, Sendable, Equatable {
    /// Nil for local drafts and previews, which the core has not delivered.
    var cursor: TranscriptCursor? = nil
    var revision: UInt64
    var conversation: Conversation
    var contextRuntimeID: String
    var modelRecordKey: String?
    var runState: RunState
    var messages: [Message]
    var transcriptTurns: [TranscriptTurn] = []
    var transcriptItems: [TranscriptItem] = []
    var transcriptOutline: [TranscriptOutlineEntry] = []
    var transcriptMessageIdentities: [TranscriptMessageIdentity] = []
    var messageIdentityConfirmations: [MessageIdentityConfirmation] = []
    var pendingInteractions: [RuntimeInteraction] = []
    var actions: ConversationActions
    enum CodingKeys: String, CodingKey { case revision, conversation, runState, messages, transcriptTurns, transcriptItems, transcriptOutline, transcriptMessageIdentities, messageIdentityConfirmations, pendingInteractions, actions; case modelRecordKey = "modelRecordKey", contextRuntimeID = "contextRuntimeId" }
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
/// What the next turn runs on: a model of the runtime's own providers, a
/// Velune-managed model, or (generic ACP) the Agent's current choice.
enum ModelSelection: Hashable, Sendable {
    case runtimeDefault
    case managed(recordKey: String)
    case native(providerID: String?, modelID: String)
}
/// A model the next turn can use, with the reasoning levels the runtime
/// accepts for it.
struct ModelChoice: Identifiable, Hashable, Sendable {
    var id: ModelSelection { selection }
    var selection: ModelSelection
    var displayName: String
    var reasoningEfforts: [String] = []
}
/// The models a runtime instance offers, read from the core when first needed.
enum RuntimeModelCatalog: Equatable, Sendable {
    case loading
    case failed(String)
    /// The runtime's default when it has one, its own models, then managed models.
    case ready([ModelChoice])
}
/// Where a runtime instance runs; paths of a remote one belong to that machine.
enum RuntimeLocation: Equatable, Sendable {
    case local
    case ssh(destination: String)
    var host: String? { if case .ssh(let destination) = self { destination } else { nil } }
}
/// An option a live session offers, as its Agent names and groups it.
struct SessionOption: Identifiable, Hashable, Sendable {
    var id: String
    var name: String
    var description: String?
    /// The Agent's kind of option, such as "model" or "mode".
    var category: String?
    var value: SessionOptionValue
    var choices: [SessionOptionChoice]
}
enum SessionOptionValue: Hashable, Sendable { case select(String), boolean(Bool) }
struct SessionOptionChoice: Hashable, Sendable {
    var value: String
    var name: String
    var description: String?
    var group: String?
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
    var enabled: Bool = true
    var id: String
    var name: String
    var typeID: String
    var gatewayID: String
    var settings: [String: String]
    enum CodingKeys: String, CodingKey { case enabled, id, name, settings; case typeID = "typeId"; case gatewayID = "gatewayId" }
}

struct RuntimeTypeDescriptor: Codable, Sendable, Identifiable, Equatable {
    var id: String
    var familyID: String
    var canRenameConversations: Bool = false
    var canDeleteConversations: Bool = false
    /// Instances take launch arguments and environment.
    var launchOptions = false
    /// The runtime can describe its Agent, capabilities and sign-in methods.
    var agentInfo = false
    /// A live conversation has options; the runtime's own models belong to that session.
    var sessionOptions = false
    /// Running over SSH needs a helper resources directory on the remote machine.
    var remoteResources = false
    var supportedProtocols: [ProviderProtocol]
    var supportedProviderProtocols: [ProviderProtocol]
    var name: String
    var fields: [SettingField]
    /// Modes a turn may request, in the runtime's own vocabulary; empty when it offers no choice.
    var permissionModes: [PermissionMode] = []
}

struct PermissionMode: Codable, Sendable, Identifiable, Equatable {
    var id: String
    var label: String
    /// What the mode allows.
    var detail: String
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

struct AuthenticationPrompt: Decodable, Sendable, Identifiable, Equatable {
    var id: String
    var kind: String
    var text: String
    var options: [SettingOption]?
}
struct AuthenticationNotification: Decodable, Sendable, Equatable {
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
    var executionInvalidated: Bool?
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

struct HistoryFailure: Sendable { var runtimeID: String; var detail: String }
