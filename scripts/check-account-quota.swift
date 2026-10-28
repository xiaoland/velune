import Foundation
import VeluneBindings

@main
struct AccountQuotaChecks {
    static func main() {
        let now = Date(timeIntervalSince1970: 1_800_000_000)
        let windows = [
            QuotaWindow(id: "window:18000", usedPercent: 54, resetsAt: now.addingTimeInterval(3600), observedAt: now),
            QuotaWindow(id: "window:604800", usedPercent: 30, resetsAt: now.addingTimeInterval(604800), observedAt: now),
            QuotaWindow(id: "model-specific", modelIDs: ["opus"], usedPercent: 20, resetsAt: nil, observedAt: now),
        ]
        let quota = AccountQuota(freshness: .fresh, windows: windows, refreshing: false)
        let claude = AIProvider(id: "claude", name: "Claude Code", protocolID: .messagesV1, endpoint: "",
                                models: [ProviderModel(recordKey: "sonnet-key", providerModelID: "sonnet"), ProviderModel(recordKey: "opus-key", providerModelID: "opus")],
                                composition: ProviderComposition(quotaSource: .claudeCliUsage))
        let codex = AIProvider(id: "codex", name: "Codex", protocolID: .responsesV1, endpoint: "",
                               models: [ProviderModel(recordKey: "gpt-key", providerModelID: "gpt")],
                               composition: ProviderComposition(quotaSource: .codexUsage))
        let gateway = GatewayConfig(providers: [claude, codex], routes: [ModelRoute(recordKey: "route", name: "混合路由", members: ["opus-key", "gpt-key", "sonnet-key"])])
        let quotas = [claude.id: quota, codex.id: quota]
        let direct = ComposerAccountQuota.providerAccounts(for: .managed(recordKey: "sonnet-key"), gateway: gateway, quotas: quotas)
        precondition(direct.compactMap(\.providerID) == ["claude"])
        precondition(direct[0].windows.map(\.id) == ["window:18000", "window:604800"], "Must retain both limits, excluding other models' windows")
        let route = ComposerAccountQuota.providerAccounts(for: .managed(recordKey: "route"), gateway: gateway, quotas: quotas)
        precondition(route.compactMap(\.providerID) == ["claude", "codex"], "Each route account appears once")
        precondition(route[0].windows == windows, "Combine model scopes across the account's route members")
        let switched = ComposerAccountQuota.providerAccounts(for: .managed(recordKey: "gpt-key"), gateway: gateway, quotas: quotas)
        precondition(switched.compactMap(\.providerID) == ["codex"], "Changing the model must change the displayed account")
        precondition(ComposerAccountQuota.providerAccounts(for: .managed(recordKey: "missing"), gateway: gateway, quotas: quotas).isEmpty)
        let runtime = RuntimeInstance(id: "native", name: "Claude Code", typeID: "claude-code-2.1", gatewayID: "default", settings: [:])
        let native = ComposerAccountQuota(source: .runtimeLogin(runtime), quota: quota)
        precondition(native.windows == windows, "Native login quota must not depend on a provider model ID")
        precondition(native.providerID == nil && native.limit(for: windows[0].id) == nil)
        let pending = ComposerAccountQuota.providerAccounts(for: .managed(recordKey: "sonnet-key"), gateway: gateway, quotas: [:])
        precondition(pending.count == 1 && pending[0].quota == nil, "Keep the account visible before its first quota observation")
        var signedOut = claude
        signedOut.authentication = ProviderAuthentication(method: .oauth, configured: false)
        precondition(ComposerAccountQuota.providerAccounts(for: .managed(recordKey: "sonnet-key"), gateway: GatewayConfig(providers: [signedOut]), quotas: quotas).isEmpty)
        var unsupported = claude
        unsupported.composition.quotaSource = .none
        precondition(ComposerAccountQuota.providerAccounts(for: .managed(recordKey: "sonnet-key"), gateway: GatewayConfig(providers: [unsupported]), quotas: quotas).isEmpty)

        let zone = TimeZone(secondsFromGMT: 8 * 3600)!
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = zone
        let reset = calendar.date(from: DateComponents(year: 2026, month: 11, day: 3, hour: 9, minute: 7))!
        let text = QuotaResetFormat.text(reset, timeZone: zone)
        precondition(text.contains("2026") && text.contains("11") && text.contains("03") && text.contains("09:07"), text)
        precondition(QuotaResetFormat.text(reset, timeZone: TimeZone(secondsFromGMT: 0)!).contains("01:07"), "Use the viewer's local timezone")
        precondition(QuotaResetFormat.text(nil) == "重置时间未知")
        print("账户额度检查通过：多窗口、模型范围、路由去重、切换账户、未知时间、本地时分。")
    }
}
