import VeluneBindings

/// Synthetic lifecycle checks, shared by the native interaction check entry.
func runtimeRecoveryLifecycleChecks() {
    func message(_ id: String, _ retry: RuntimeRetry?) -> DeliveredMessage {
        DeliveredMessage(stamp: 0, completed: true, id: id, role: .system, timestampUnixMs: nil, blocks: [.runtimeFailure(failure: .other, detail: "合成断连", retry: retry)])
    }
    let pending = RuntimeRecovery.latest(in: [message("first", .native)])!
    precondition(pending.active && pending.isVisible(dismissed: nil))
    precondition(!pending.isVisible(dismissed: pending.dismissalID), "忽略提示不等于停止重试")
    let recovered = message("first", .recovered)
    precondition(RuntimeRecovery.latest(in: [recovered]) == nil, "恢复后自动撤下提示")
    precondition(RuntimeRecovery.latest(in: [message("older", nil), recovered]) == nil, "恢复后不能重新显示更早错误")
    let next = RuntimeRecovery.latest(in: [recovered, message("second", .native)])!
    precondition(next.isVisible(dismissed: pending.dismissalID), "同一轮再次断连必须重新显示")
    for retry: RuntimeRetry? in [nil, .cancelled, .exhausted(attempts: 5)] {
        let ended = RuntimeRecovery.latest(in: [message("first", retry)])!
        precondition(!ended.active && ended.isVisible(dismissed: pending.dismissalID), "终止结果不能被先前忽略隐藏")
        precondition(!ended.isVisible(dismissed: ended.dismissalID))
    }
    let user = DeliveredMessage(stamp: 0, completed: true, id: "new-turn", role: .user, timestampUnixMs: nil, blocks: [.text(text: "合成新轮次")])
    precondition(RuntimeRecovery.latest(in: [message("old", nil), user]) == nil)
}
