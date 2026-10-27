/// One submission intent for the live editor. Runtime eligibility stays in
/// the Store; draft and input-method state belong to this tab's composer.
struct ComposerSubmission: Equatable {
    enum Phase { case idle, running, stopping, sending, resuming, blocked }
    enum Action {
        case send, queue, resume, stopping, sending, resuming, none
        var title: String {
            switch self {
            case .send: "发送"
            case .queue: "排队"
            case .resume: "继续"
            case .stopping: "正在停止…"
            case .sending: "正在发送…"
            case .resuming: "正在恢复…"
            case .none: ""
            }
        }
    }
    let action: Action
    let enabled: Bool

    init(phase: Phase, hasInput: Bool, isComposing: Bool, canSend: Bool, canQueue: Bool, canResume: Bool) {
        switch phase {
        case .stopping: action = .stopping; enabled = false
        case .sending: action = .sending; enabled = false
        case .resuming: action = .resuming; enabled = false
        case .blocked: action = .send; enabled = false
        case .running:
            action = hasInput ? .queue : .none
            enabled = hasInput && canQueue && !isComposing
        case .idle:
            action = hasInput ? .send : canResume ? .resume : .send
            enabled = !isComposing && canSend && (hasInput || canResume)
        }
    }
    var canSubmitDraft: Bool { enabled && (action == .send || action == .queue) }
    var canResume: Bool { enabled && action == .resume }
}
