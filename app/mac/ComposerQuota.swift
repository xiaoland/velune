import Foundation
import VeluneBindings

/// A presentation value projected by application; no provider or runtime configuration.
struct ComposerAccountQuota: Identifiable {
    let id: String
    let name: String
    let origin: QuotaOrigin
    let quota: AccountQuota
    let limits: [QuotaLimit]
    var title: String {
        switch origin {
        case .provider: name
        case .runtimeLogin: "\(name) · 登录额度"
        case .conversationRecord: "\(name) · 会话记录中的限额"
        }
    }
    var windows: [QuotaWindow] { quota.windows }
    var isRecorded: Bool { if case .conversationRecord = origin { true } else { false } }
    func limit(for windowID: String) -> UInt8? { limits.first { $0.windowId == windowID }?.maxUsedPercent }
}

/// Captures intent and view revisions so late replies cannot replace a newer selection.
struct ComposerQuotaRequest: Equatable {
    let runtimeID: String
    let selection: ModelSelection?
    let conversationID: String?
    let conversationRevision: UInt64?
    let configurationRevision: UInt64
}

/// Absolute local time remains precise even for resets several days away.
enum QuotaResetFormat {
    static func text(_ date: Date?, timeZone: TimeZone = .current) -> String {
        guard let date else { return "重置时间未知" }
        var format = Date.FormatStyle.dateTime.year().month(.twoDigits).day(.twoDigits).hour(.twoDigits(amPM: .omitted)).minute(.twoDigits)
            .locale(Locale(identifier: "zh_CN"))
        format.timeZone = timeZone
        return "\(date.formatted(format)) 重置"
    }
}
