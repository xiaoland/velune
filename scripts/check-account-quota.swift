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
        let now = Date(timeIntervalSince1970: 1_800_000_000)
        let windows = [
            QuotaWindow(id: "window:18000", usedPercent: 54, resetsAt: now.addingTimeInterval(3600), observedAt: now),
            QuotaWindow(id: "window:604800", usedPercent: 30, resetsAt: now.addingTimeInterval(604800), observedAt: now),
            QuotaWindow(id: "model-specific", modelIDs: ["opus"], usedPercent: 20, resetsAt: nil, observedAt: now),
        ]
        let quota = AccountQuota(freshness: .fresh, windows: windows, refreshing: false)
        let native = ComposerAccountQuota(id: "login", name: "运行时", origin: .runtimeLogin, quota: quota, limits: [])
        try require(native.windows == windows && native.limit(for: windows[0].id) == nil)
        try require(native.title == "运行时 · 登录额度" && !native.isRecorded)
        let recorded = ComposerAccountQuota(id: "record", name: "运行时", origin: .conversationRecord, quota: quota, limits: [])
        try require(recorded.isRecorded && recorded.title.contains("会话记录"))

        let zone = TimeZone(secondsFromGMT: 8 * 3600)!
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = zone
        let reset = calendar.date(from: DateComponents(year: 2026, month: 11, day: 3, hour: 9, minute: 7))!
        let text = QuotaResetFormat.text(reset, timeZone: zone)
        try require(text.contains("2026") && text.contains("11") && text.contains("03") && text.contains("09:07"), text)
        try require(QuotaResetFormat.text(reset, timeZone: TimeZone(secondsFromGMT: 0)!).contains("01:07"), "Use the viewer's local timezone")
        try require(QuotaResetFormat.text(nil) == "重置时间未知")
        print("账户额度检查通过：通用额度条目、记录来源、未知时间、本地时分。")
    }
}
