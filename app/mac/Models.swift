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
    /// Velune's organization of the conversation in the list; see `ListedConversation`.
    var nativeID = ""
    var agentsURI: String? = nil
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
    var group: ConversationGroupKey? = nil
    enum CodingKeys: String, CodingKey { case id, title, updatedAtUnixMs, createdAtUnixMs, cwd, canRename, canDelete; case runtimeID = "runtimeId"; case parentConversationID = "parentConversationId" }
    /// This conversation's runtime fields with `listed`'s organization.
    func organized(like listed: Conversation) -> Conversation {
        var value = self
        value.nativeID = listed.nativeID; value.pinned = listed.pinned; value.unread = listed.unread; value.archived = listed.archived
        value.agentsURI = listed.agentsURI
        value.project = listed.project; value.moved = listed.moved; value.failed = listed.failed; value.group = listed.group
        return value
    }
}








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
    /// The message as copied whole: its text and notices, a blank line apart.
    var copiedText: String { blocks.compactMap { block in switch block { case .text(let text), .notice(let text): return text; case .reasoning, .tool, .image, .file, .agentSender: return nil } }.joined(separator: "\n\n") }
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



/// The models a runtime instance offers, read from the core when first needed.
enum RuntimeModelCatalog: Equatable, Sendable {
    case loading
    case failed(String)
    /// The runtime's default when it has one, its own models, then managed models.
    case ready([RuntimeModel])
}
/// Where a runtime instance runs; paths of a remote one belong to that machine.
enum RuntimeLocation: Equatable, Sendable {
    case local
    case ssh(destination: String)
    var host: String? { if case .ssh(let destination) = self { destination } else { nil } }
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
/// What a new provider starts from, from the core's catalog. A subscription
/// fixes its protocol, address and composition; an API template stays editable.
struct ProviderTemplate: Sendable, Identifiable, Equatable {
    enum Kind: Sendable, Equatable { case subscription, api }
    var id: String
    var kind: Kind
    var name: String
    var summary: String
    var protocolID: ApiProtocol
    var endpoint: String
    var composition: ProviderComposition
    /// A local command to locate and fill into the composition's executable.
    var executableCommand: String?
    var disclosure: SubscriptionDisclosure?
}
struct AIProvider: Sendable, Identifiable, Equatable {
    var id: String
    var name: String
    var protocolID: ApiProtocol
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
    var executionBackend: ExecutionBackend = .http
    var requestProfile: RequestProfile = .native
    var authenticationSource: ProviderAuthenticationSource = .apiKey
    var quotaSource: ProviderQuotaSource = .none
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


/// One usage window as the provider last reported it.
struct QuotaWindow: Sendable, Equatable {
    var id: String
    /// The provider model IDs the window applies to; `nil` for all of the provider's models.
    var modelIDs: [String]?
    var usedPercent: Double
    var resetsAt: Date?
    /// How long one period lasts, when the provider states it.
    var period: TimeInterval?
    var observedAt: Date?
    /// Use against an even spend over the period, when both its reset and length are known.
    var pace: QuotaPace?
    func applies(to providerModelID: String) -> Bool { modelIDs?.contains(providerModelID) ?? true }
    /// The window as a row names it: a period's window, or the limit itself.
    static func title(_ id: String) -> String {
        id == "openrouter:keyLimit" ? "Key 花费上限" : "\(name(id))窗口"
    }
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
/// The observed use against spending the window evenly; the rate is the
/// average since the period began, not a forecast.
struct QuotaPace: Sendable, Equatable {
    enum Status: Sendable, Equatable { case onPace, runsOutBeforeReset, exhausted }
    var expectedUsedPercent: Double
    /// When the window empties at that rate, only if before the reset.
    var runsOutAt: Date?
    var status: Status
}
/// An amount the provider's account reports, such as a prepaid balance.
struct QuotaBalance: Sendable, Equatable {
    var id: String
    var amount: Double
    /// An ISO 4217 currency code.
    var unit: String
    var observedAt: Date
    var title: String {
        switch id {
        case "openrouter:keyLimitRemaining": return "Key 剩余额度"
        case "balance": return "可用余额"
        default: return id.hasPrefix("balance:") ? "余额（\(unit)）" : id
        }
    }
    var formatted: String { amount.formatted(.currency(code: unit).presentation(.narrow)) }
}
/// Account usage from a provider or native login. The core owns sampling;
/// this is a read-only copy for display, with its source held separately.
struct AccountQuota: Sendable, Equatable {
    var freshness: QuotaFreshness
    var windows: [QuotaWindow]
    var balances: [QuotaBalance] = []
    /// `false` when the provider says the account may not be used now, whatever the percentages.
    var available: Bool?
    var refreshing: Bool
    var problem: String?
    var observedAt: Date? { (windows.compactMap(\.observedAt) + balances.map(\.observedAt)).max() }
}


struct GatewayConfig: Sendable, Equatable {
    var id: String = "default"
    var name: String = "默认网关"
    var providers: [AIProvider] = []
    var routes: [ModelRoute] = []
}










struct AuthenticationPrompt: Decodable, Sendable, Identifiable, Equatable {
    var id: String
    var kind: String
    var text: String
    var options: [SettingOption]?
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
    var `protocol`: ApiProtocol
    var endpoint: String
    var explicitOutputCap: Bool
    var temperature: Bool
}
struct AuthenticationInspection: Decodable, Sendable { var metadata: AuthenticationMetadata }


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
        ModelTemplate(id: "", name: "\(name) · \(sourceProviderName) (models.dev)", suggestedProviderModelId: modelID, nickname: name, contextWindow: contextWindow, maxOutputTokens: maxOutputTokens, reasoningLevels: reasoningLevels)
    }
}

struct HistoryFailure: Sendable { var runtimeID: String; var detail: String }
