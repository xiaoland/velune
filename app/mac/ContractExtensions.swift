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
