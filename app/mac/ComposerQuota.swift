import Foundation
import VeluneBindings

/// A presentation value projected by application; no provider or runtime configuration.
extension ComposerQuotaEntry: Identifiable {
    var title: String {
        switch origin {
        case .provider: name
        case .runtimeLogin: "\(name)的登录额度"
        case .conversationRecord: "\(name)会话记录中的限额"
        }
    }
    var quota: QuotaObservation { observation }
    var windows: [QuotaWindowObservation] { observation.windows }
    var isRecorded: Bool { if case .conversationRecord = origin { true } else { false } }
    func limit(for windowID: String) -> UInt8? { limits.first { $0.windowId == windowID }?.maxUsedPercent }
}

extension ComposerQuotaCredits {
    private var balance: String? {
        guard let value = observation?.balance?.trimmingCharacters(in: .whitespacesAndNewlines),
              !value.isEmpty else { return nil }
        return value
    }

    var detailValue: String {
        if let value = observation?.unlimited == true ? "不限量" : balance {
            return stale ? "\(value)（上次余额）" : value
        }
        if failed { return "暂时无法读取" }
        return refreshing ? "读取中…" : "未报告"
    }

    var compactValue: String {
        if observation?.unlimited != true, let balance {
            return "\(balance) credits" + (stale ? "（上次）" : "")
        }
        return "credits \(detailValue)"
    }
}

/// Captures intent and view revisions so late replies cannot replace a newer selection.
struct ComposerQuotaRequest: Equatable {
    let runtimeID: String
    let selection: ModelSelection?
    let conversationID: String?
    let conversationRevision: UInt64?
    let configurationRevision: UInt64
    let providerReportRevision: UInt64
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
