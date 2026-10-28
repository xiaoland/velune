import Foundation
import VeluneBindings

// Mac conveniences on the generated contract types. The types themselves come
// from the Rust contract crates; add only presentation helpers here.

extension FollowUp {
    /// Nothing queued and nothing offered; previews and drafts have no follow-up.
    init() {
        self.init(queue: [], paused: nil, waiting: nil, canStopSubAgents: false, canSteer: false, canContinue: false,
                  canResume: false, canCompact: false, isCompacting: false, editable: [])
    }
    /// The next message needs another runtime process; the queue sends it by
    /// itself once the conversation's sub-agents have ended.
    var waitingForSubAgents: Bool { waiting == .subAgents }
}

extension QueuedMessage: Identifiable {}

extension QueuePause {
    var explanation: String {
        switch self {
        case .turnStopped: "上一轮已停止，队列暂停"
        case .turnFailed: "上一轮失败，队列暂停"
        case .sendFailed: "第一条未能发出，队列暂停"
        }
    }
}

extension TranscriptTurn: Identifiable {}
extension TranscriptOutlineEntry: Identifiable {}

extension RuntimeModel: Identifiable {
    public var id: ModelSelection { selection }
}

extension AcpConfigOption: Identifiable {}

extension AcpConfigChoice {
    /// The group a picker files the choice under.
    var groupLabel: String? { groupName ?? group }
}

extension ProviderModel: Identifiable {
    public var id: String { recordKey }
    var displayName: String { nickname.isEmpty ? providerModelId : nickname }
}

extension ModelTemplate: Identifiable {
    var model: ProviderModel {
        ProviderModel(recordKey: "", providerModelId: suggestedProviderModelId, nickname: nickname, icon: icon, contextWindow: contextWindow,
                      maxOutputTokens: maxOutputTokens, reasoningLevels: reasoningLevels)
    }
}

extension ApiProtocol: CaseIterable, Identifiable {
    public static var allCases: [ApiProtocol] { [.chatCompletions, .responses, .messages] }
    public var id: Self { self }
}

extension ModelRoute: Identifiable {
    public var id: String { recordKey }
}

extension RoutingStrategy: CaseIterable, Identifiable {
    public static var allCases: [RoutingStrategy] { [.smart, .order, .rotate, .weight] }
    public var id: Self { self }
    var label: String {
        switch self { case .smart: "智能"; case .order: "顺序"; case .rotate: "轮询"; case .weight: "权重" }
    }
    var explanation: String {
        switch self {
        case .smart: "优先使用重置前更可能用不完的额度：比较各提供商最紧的额度窗口还剩的比例与均匀消耗时此刻应剩的比例，临近重置仍有余量的先用；未报告重置周期的窗口只比较剩余比例。额度未知的排在后面，相同时按列表顺序。"
        case .order: "按列表顺序使用，前面的模型不可用时才用后面的。"
        case .rotate: "在模型之间轮流分配。"
        case .weight: "按权重比例分配，权重越大分到越多。"
        }
    }
}

extension RuntimeInstance: Identifiable {}
extension RuntimeTypeDescriptor: Identifiable {}
extension PermissionMode: Identifiable {}
extension ProtocolDescriptor: Identifiable {}
extension SettingOption: Identifiable {}
extension SettingAction: Identifiable {}
extension SettingField: Identifiable {
    public var id: String { key }
}
extension RuntimeInteraction: Identifiable {}
extension InteractionOption: Identifiable {}
extension InteractionQuestion: Identifiable {}

extension ImportProviderCandidate: Identifiable {}
extension ImportModelCandidate: Identifiable {
    public var id: String { candidateKey }
}

extension SubAgent: Identifiable {
    public var id: String { conversationId }
}

extension Collaborator: Identifiable {
    public var id: String { conversationId }
}

extension RuntimeTransport {
    /// The SSH destination; paths of a remote runtime belong to that machine.
    var host: String? { if case .ssh(_, let destination, _) = self { destination } else { nil } }
}

extension DeliveredMessage: Identifiable {
    /// Visible message text; reasoning and tool output are separate disclosures.
    var text: String { blocks.compactMap(\.visibleText).joined() }
    /// The message as copied whole: its text and notices, a blank line apart.
    var copiedText: String { blocks.compactMap(\.visibleText).joined(separator: "\n\n") }
    var hasDeferredPayload: Bool {
        blocks.contains { block in
            switch block {
            case .reasoning(_, .deferred), .tool(_, _, _, .deferred?, _), .image(_, .data(.deferred)): return true
            case .text, .notice, .contextCompaction, .reasoning(_, .inline), .tool, .image, .file, .agentSender: return false
            }
        }
    }
    /// An image whose content is still to be read for its thumbnail.
    var hasDeferredImage: Bool { blocks.contains { if case .image(_, .data(.deferred)) = $0 { return true }; return false } }
}

extension DeliveredBlock {
    /// Text the message shows inline: its text and notices.
    var visibleText: String? {
        switch self {
        case .text(let text), .notice(let text), .contextCompaction(let text): text
        case .reasoning, .tool, .image, .file, .agentSender: nil
        }
    }
}

extension DeliveredMessage {
    /// A message the Mac shows before the core delivers it: a pending send or a preview.
    init(id: String, role: MessageRole, blocks: [DeliveredBlock]) {
        self.init(stamp: 0, completed: true, id: id, role: role, timestampUnixMs: nil, blocks: blocks)
    }
}

extension ProviderSummary: Identifiable {
    var composition: ProviderComposition {
        ProviderComposition(executionBackend: executionBackend, requestProfile: requestProfile, authenticationSource: authenticationSource, quotaSource: quotaSource)
    }
    /// A provider the design preview saves without the core: it has not signed in.
    init(previewOf draft: ProviderDraft) {
        self.init(id: draft.id, name: draft.name, protocol: draft.protocol, endpoint: draft.endpoint, executionBackend: draft.executionBackend,
                  requestProfile: draft.requestProfile, authenticationSource: draft.authenticationSource, quotaSource: draft.quotaSource,
                  authentication: .unconfigured, quotaAuthentication: QuotaAuthenticationDescription(configured: false, bindingConfirmed: false),
                  quotaLimits: draft.quotaLimits, models: draft.models)
    }
}

extension ProviderTemplate: Identifiable {
    var composition: ProviderComposition {
        ProviderComposition(executionBackend: executionBackend, requestProfile: requestProfile, authenticationSource: authenticationSource, quotaSource: quotaSource)
    }
}

extension GatewaySummary: Identifiable {}

extension ProviderAuthenticationDescription {
    static let unconfigured = ProviderAuthenticationDescription(method: .unconfigured, configured: false, accountLabel: nil, provenance: nil, actions: [], disclosure: nil, linkedLogin: nil)
    /// Signing in, for the sources that offer it.
    var loginAction: SettingAction? { actions.first { $0.id == "login" } }
    /// A login method that has not signed in, or whose login lapsed.
    var awaitsSignIn: Bool { method == .oAuth && !configured }
}

extension AuthenticationMethod {
    var label: String {
        switch self { case .apiKey: "API key"; case .oAuth: "OAuth"; case .unconfigured: "尚未配置" }
    }
}

private extension Int64 {
    var unixMsDate: Date { Date(timeIntervalSince1970: Double(self) / 1000) }
}

/// Account usage from a provider or native login. The core owns sampling;
/// this is a read-only copy for display, with its source held separately.
extension QuotaObservation {
    var observedAt: Date? { (windows.compactMap(\.observedAt) + balances.map(\.observedAt)).max() }
}

extension QuotaWindowObservation {
    var resetsAt: Date? { resetAtMs?.unixMsDate }
    /// How long one period lasts, when the provider states it.
    var period: TimeInterval? { windowMs.map { Double($0) / 1000 } }
    var observedAt: Date? { observedAtMs?.unixMsDate }
    func applies(to providerModelID: String) -> Bool { modelIds?.contains(providerModelID) ?? true }
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
extension QuotaPaceObservation {
    /// When the window empties at that rate, only if before the reset.
    var runsOutAt: Date? { runsOutAtMs?.unixMsDate }
}

/// An amount the provider's account reports, such as a prepaid balance.
extension QuotaBalanceObservation {
    var observedAt: Date { observedAtMs.unixMsDate }
    var title: String { Self.title(id: id, unit: unit) }
    static func title(id: String, unit: String) -> String {
        switch id {
        case "openrouter:keyLimitRemaining": return "Key 剩余额度"
        case "balance": return "可用余额"
        default: return id.hasPrefix("balance:") ? "余额（\(unit)）" : id
        }
    }
    var formatted: String { amount.formatted(.currency(code: unit).presentation(.narrow)) }
}
