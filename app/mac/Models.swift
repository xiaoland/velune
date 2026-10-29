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

struct ConversationSnapshot: Codable, Sendable, Equatable {
    /// Nil for local drafts and previews, which the core has not delivered.
    var cursor: TranscriptCursor? = nil
    var revision: UInt64
    var conversation: Conversation
    var contextRuntimeID: String
    var modelRecordKey: String?
    var runState: RunState
    var messages: [DeliveredMessage]
    var transcriptTurns: [TranscriptTurn] = []
    /// What the turns' native records say the conversation used, as the core sums it.
    var usage = ConversationUsage()
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
    enum CodingKeys: String, CodingKey { case revision, conversation, runState, messages, transcriptTurns, usage, transcriptItems, transcriptOutline, transcriptMessageIdentities, messageIdentityConfirmations, pendingInteractions, actions; case modelRecordKey = "modelRecordKey", contextRuntimeID = "contextRuntimeId" }
}

/// The models a runtime instance offers, read from the core when first needed.
enum RuntimeModelCatalog: Equatable, Sendable {
    case loading
    case failed(String)
    /// The runtime's default when it has one, its own models, then managed models.
    case ready([RuntimeModel])

    var models: [RuntimeModel]? { if case .ready(let models) = self { models } else { nil } }
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

struct PendingAuthenticationPrompt: Sendable, Identifiable, Equatable {
    var id: String
    var kind: String
    var text: String
    var options: [SettingOption]?
}

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
