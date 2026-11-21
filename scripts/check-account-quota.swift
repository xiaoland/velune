import Foundation
import VeluneBindings

@main
struct AccountQuotaChecks {
    static func main() {
        do { try run() }
        catch { FileHandle.standardError.write(Data("FAIL: \(error)\n".utf8)); exit(1) }
    }
    static func require(_ condition: @autoclosure () -> Bool, _ message: String = "额度呈现契约不符") throws {
        if !condition() { throw NSError(domain: "AccountQuotaChecks", code: 1, userInfo: [NSLocalizedDescriptionKey: message]) }
    }
    static func run() throws {
        let windows = [
            QuotaWindowObservation(id: "5h", modelIds: nil, usedPercent: 54, resetAtMs: 1_800_003_600_000,
                                   windowMs: 18_000_000, observedAtMs: 1_800_000_000_000, expiresAtMs: nil, pace: nil),
            QuotaWindowObservation(id: "opus", modelIds: ["opus"], usedPercent: 20, resetAtMs: nil,
                                   windowMs: nil, observedAtMs: 1_800_000_000_000, expiresAtMs: nil, pace: nil),
        ]
        let observation = QuotaObservation(freshness: .fresh, windows: windows, balances: [],
                                           available: true, refreshing: false, problem: nil)
        let native = ComposerQuotaEntry(id: "login", name: "运行时", origin: .runtimeLogin, observation: observation,
                                        limits: [QuotaLimit(windowId: "5h", maxUsedPercent: 80)], credits: nil)
        try require(native.windows == windows && native.limit(for: "5h") == 80 && native.limit(for: "opus") == nil)
        try require(native.title == "运行时的登录额度" && !native.isRecorded)
        let recorded = ComposerQuotaEntry(id: "record", name: "运行时", origin: .conversationRecord,
                                          observation: observation, limits: [], credits: nil)
        try require(recorded.isRecorded && recorded.title == "运行时会话记录中的限额")

        let zone = TimeZone(secondsFromGMT: 8 * 3600)!
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = zone
        let reset = calendar.date(from: DateComponents(year: 2026, month: 11, day: 3, hour: 9, minute: 7))!
        let text = QuotaResetFormat.text(reset, timeZone: zone)
        try require(text.contains("2026") && text.contains("11") && text.contains("03") && text.contains("09:07"), text)
        try require(QuotaResetFormat.text(reset, timeZone: TimeZone(secondsFromGMT: 0)!).contains("01:07"), "Use the viewer's local timezone")
        try require(QuotaResetFormat.text(nil) == "重置时间未知")

        let zero = ComposerQuotaCredits(observation: ProviderAccountCredits(hasCredits: false, unlimited: false, balance: "0"),
                                       stale: false, refreshing: false, failed: false)
        let unlimited = ComposerQuotaCredits(observation: ProviderAccountCredits(hasCredits: true, unlimited: true, balance: nil),
                                            stale: true, refreshing: false, failed: false)
        let missing = ComposerQuotaCredits(observation: nil, stale: false, refreshing: false, failed: false)
        let failed = ComposerQuotaCredits(observation: nil, stale: false, refreshing: false, failed: true)
        try require(zero.detailValue == "0" && zero.compactValue == "0 credits")
        try require(unlimited.detailValue == "不限量（上次余额）")
        try require(missing.detailValue == "未报告" && failed.detailValue == "暂时无法读取")
        print("账户额度检查通过，覆盖额度条目、记录来源、重置时间和 credits 余额。")
    }
}
