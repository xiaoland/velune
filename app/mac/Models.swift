import Foundation
import VeluneBindings

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
    /// Velune's organization of the conversation in the list; see `BindingListedConversation`.
    var nativeID = ""
    var pinned = false
    var unread = false
    var archived = false
    /// Where the list files it: where it was moved, else its working
    /// directory, unless the runtime started it there without a project.
    var project: String? = nil
    var moved = false
    /// Its latest turn failed; known for unread conversations only.
    var failed = false
    /// The list group showing it; nil for an open conversation the list does not show.
    var group: BindingConversationGroupKey? = nil
    enum CodingKeys: String, CodingKey { case id, title, updatedAtUnixMs, createdAtUnixMs, cwd, canRename, canDelete; case runtimeID = "runtimeId"; case parentConversationID = "parentConversationId" }
    /// This conversation's runtime fields with `listed`'s organization.
    func organized(like listed: Conversation) -> Conversation {
        var value = self
        value.nativeID = listed.nativeID; value.pinned = listed.pinned; value.unread = listed.unread; value.archived = listed.archived
        value.project = listed.project; value.moved = listed.moved; value.failed = listed.failed; value.group = listed.group
        return value
    }
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

/// Messages waiting for the running turn to end, and what the user may do next.
struct FollowUp: Sendable, Equatable {
    var queue: [QueuedMessage] = []
    var paused: QueuePause?
    /// The next message needs another runtime process; the queue sends it
    /// by itself once the conversation's sub-agents have ended.
    var waitingForSubAgents = false
    /// Sub-agents the conversation started still run and may be stopped.
    var canStopSubAgents = false
    var canSteer = false
    var canContinue = false
    /// User messages that may be edited now.
    var editable: [EditableMessage] = []
}

/// A user message an edit can replace, continuing from just before it.
struct EditableMessage: Sendable, Equatable {
    var messageID: String
    /// The replacement may go into this conversation; otherwise only a new
    /// conversation can continue from the message.
    var inPlace: Bool
}

struct QueuedMessage: Sendable, Equatable, Identifiable {
    var id: String
    var text: String
    var attachments: [String]
}

enum QueuePause: Sendable, Equatable {
    case turnStopped, turnFailed, sendFailed
    var explanation: String {
        switch self {
        case .turnStopped: "上一轮已停止，队列暂停"
        case .turnFailed: "上一轮失败，队列暂停"
        case .sendFailed: "第一条未能发出，队列暂停"
        }
    }
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
    /// The model Velune sent the turn with, when it sent it and named one.
    var modelSelection: ModelSelection?
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

/// A native sub-agent of a conversation: its own conversation, opened read-only.
struct SubAgent: Codable, Sendable, Equatable, Identifiable {
    var id: String
    var title: String
    /// Its last turn failed, as last listed.
    var failed: Bool
}

/// A conversation that took part with this one in Velune's agent-to-agent
/// collaboration: an ordinary conversation, opened and continued as any other.
struct Collaborator: Codable, Sendable, Equatable, Identifiable {
    var id: String
    /// Empty when it is neither listed nor named by its message.
    var title: String
    var runtimeName: String?
    /// This conversation's agent started or addressed it.
    var contacted: Bool
    /// Its agent sent this conversation a message.
    var contactedBy: Bool
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
    /// Velune's queue and next actions; previews and drafts have none.
    var followUp = FollowUp()
    /// Native sub-agents; whether one runs is conversation activity.
    var subAgents: [SubAgent] = []
    /// Conversations of Velune's agent-to-agent collaboration, either way.
    var collaborators: [Collaborator] = []
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
    var displayName: String { nickname.isEmpty ? providerModelID : nickname }
}
/// What the next turn runs on: a model of the runtime's own providers or a
/// Velune-managed model. No selection (`nil`) is the runtime's own default;
/// Velune never names a default of its own.
enum ModelSelection: Hashable, Sendable, Codable {
    case managed(recordKey: String)
    case native(providerID: String?, modelID: String)
}
/// What the user chose for a turn on one runtime. Each `nil` part follows the
/// conversation's current one, and where there is none (a conversation not
/// yet sent on the runtime) is the runtime's own default.
struct TurnChoice: Hashable, Sendable {
    var model: ModelSelection?
    var effort: String?
    var permissionMode: String?
}
/// What a runtime instance's new conversations start with.
enum NewConversationChoice: Hashable, Sendable {
    /// The user's last choice on the runtime.
    case lastChosen
    /// These; a `nil` part is the runtime's own default.
    case chosen(TurnChoice)
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
    /// The tail of the bound account ID, which tells several logins apart; kept after the login lapses.
    var accountLabel: String? = nil
    var provenance: String? = nil
    /// Present for subscription authentication, whose terms the user should see.
    var disclosure: SubscriptionDisclosure? = nil
    var actions: [SettingAction] = []
    /// The existing Claude Code login this provider references, if it does.
    var linkedLogin: LinkedClaudeLogin? = nil
    /// Signing in, for the sources that offer it.
    var loginAction: SettingAction? { actions.first { $0.id == "login" } }
    /// A login method that has not signed in, or whose login lapsed.
    var awaitsSignIn: Bool { method == .oauth && !configured }
}
struct SubscriptionDisclosure: Codable, Sendable, Equatable {
    var title: String
    var detail: String
}
/// What a new provider starts from, from the core's catalog. A subscription
/// fixes its protocol, address and composition; an API template stays editable.
struct ProviderTemplate: Sendable, Identifiable, Equatable {
    enum Kind: Sendable, Equatable { case subscription, api }
    var id: String
    var kind: Kind
    var name: String
    var summary: String
    var protocolID: ProviderProtocol
    var endpoint: String
    var composition: ProviderComposition
    /// A local command to locate and fill into the composition's executable.
    var executableCommand: String?
    var disclosure: SubscriptionDisclosure?
}
/// A Claude Code login the user already has; the official CLI keeps it, and Velune never signs it out.
enum LinkedClaudeLogin: Codable, Sendable, Hashable { case userDefault, runtime(instanceID: String) }
enum AuthenticationEdit: Sendable { case keep, setAPIKey(String), clear, useClaudeCodeLogin(LinkedClaudeLogin) }
struct AIProvider: Sendable, Identifiable, Equatable {
    var id: String
    var name: String
    var protocolID: ProviderProtocol
    var endpoint: String
    var authentication: ProviderAuthentication = ProviderAuthentication()
    var models: [ProviderModel]
    /// Used-percent ceilings per quota window, which smart scheduling admits a request under.
    var quotaLimits: [QuotaLimit] = []
    var composition = ProviderComposition()
    var quotaAuthenticationConfigured = false
    var quotaBindingConfirmed = false
}

/// How core reaches the provider; template kind does not determine its authentication method.
struct ProviderComposition: Sendable, Equatable {
    var executionBackend: BindingExecutionBackend = .http
    var requestProfile: BindingRequestProfile = .native
    var authenticationSource: BindingProviderAuthenticationSource = .apiKey
    var quotaSource: BindingProviderQuotaSource = .none
    /// The subscription composition's display name, independently of how it authenticates.
    var subscriptionName: String? {
        if requestProfile == .kimiCoding { return "Kimi Coding Plan" }
        if requestProfile == .arkCoding { return "ARK Coding Plan" }
        switch authenticationSource {
        case .codexOAuth: return "Codex 订阅"
        case .claudeCli: return "Claude Code 订阅"
        case .none, .apiKey, .runtimeProvider: return nil
        }
    }
    /// How signing in proceeds once a new subscription provider is saved.
    var signInHint: String? {
        switch authenticationSource {
        case .codexOAuth: return "保存后在浏览器中登录一个 ChatGPT 账户。"
        case .claudeCli: return "保存后由 Claude Code CLI 在浏览器中登录一个 Claude 账户。"
        case .none, .apiKey, .runtimeProvider: return nil
        }
    }
    /// The Claude Code CLI executable, when the CLI executes the provider's requests.
    var claudeBinary: String? {
        get { if case .claudeCli(let binary) = executionBackend { return binary }; return nil }
        set { if let newValue { executionBackend = .claudeCli(binary: newValue) } }
    }
}

struct QuotaLimit: Codable, Sendable, Equatable {
    /// A window ID reported by the provider's quota status.
    var windowID: String
    var maxUsedPercent: UInt8
}

enum QuotaFreshness: Sendable, Equatable { case fresh, stale, unknown, unsupported }
/// One usage window as the provider last reported it.
struct QuotaWindow: Sendable, Equatable {
    var id: String
    /// The provider model IDs the window applies to; `nil` for all of the provider's models.
    var modelIDs: [String]?
    var usedPercent: Double
    var resetsAt: Date?
    var observedAt: Date
    func applies(to providerModelID: String) -> Bool { modelIDs?.contains(providerModelID) ?? true }
    /// A readable name for a window ID; Codex names its windows `window:<seconds>`.
    static func name(_ id: String) -> String {
        switch id {
        case "ark:session": return "5 小时"
        case "ark:weekly": return "周"
        case "ark:monthly": return "订阅月"
        case "kimi:limit_5h": return "5 小时"
        case "kimi:limit_7d": return "7 天"
        case "kimi:limit_month_total": return "会员月度总额度"
        case "kimi:limit_month_code": return "月度编程额度"
        default: break
        }
        guard id.hasPrefix("window:"), let seconds = Int(id.dropFirst("window:".count)), seconds > 0 else { return id }
        if seconds % 86_400 == 0 { return "\(seconds / 86_400) 天" }
        if seconds % 3_600 == 0 { return "\(seconds / 3_600) 小时" }
        return "\(max(1, seconds / 60)) 分钟"
    }
}
/// A provider's usage as the core last observed it. The core owns sampling;
/// this is a read-only copy for display.
struct ProviderQuota: Sendable, Equatable {
    var freshness: QuotaFreshness
    var windows: [QuotaWindow]
    /// `false` when the provider says the account may not be used now, whatever the percentages.
    var available: Bool?
    var refreshing: Bool
    var problem: String?
    var observedAt: Date? { windows.map(\.observedAt).max() }
}

enum FailoverMode: String, Codable, Sendable { case disabled }
struct FailoverPolicy: Codable, Sendable, Equatable { var mode: FailoverMode = .disabled }
/// Provider models chosen together as one model; the gateway picks among
/// them by the strategy and fails over before a response starts.
struct ModelRoute: Codable, Sendable, Identifiable, Equatable {
    var recordKey: String
    var name: String
    /// Provider model record keys; the order breaks ties and is the order strategy itself.
    var members: [String]
    var strategy: RoutingStrategy = .smart
    /// A weight per member, for the weight strategy alone.
    var weights: [String: UInt32] = [:]
    var sticky = true
    var id: String { recordKey }
}

enum RoutingStrategy: String, Codable, Sendable, CaseIterable, Identifiable {
    case smart, order, rotate, weight
    var id: String { rawValue }
    var label: String {
        switch self { case .smart: return "智能"; case .order: return "顺序"; case .rotate: return "轮询"; case .weight: return "权重" }
    }
    var explanation: String {
        switch self {
        case .smart: return "优先使用离使用上限最远的模型：比较各提供商最紧的额度窗口还剩的比例；额度未知的排在后面，相同时按列表顺序。"
        case .order: return "按列表顺序使用，前面的模型不可用时才用后面的。"
        case .rotate: return "在模型之间轮流分配。"
        case .weight: return "按权重比例分配，权重越大分到越多。"
        }
    }
}
struct GatewayConfig: Sendable, Equatable {
    var id: String = "default"
    var name: String = "默认网关"
    var providers: [AIProvider] = []
    var routes: [ModelRoute] = []
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
struct AuthenticationData: Sendable {
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
