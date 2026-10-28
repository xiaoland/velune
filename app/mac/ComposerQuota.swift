import Foundation
import VeluneBindings

/// Quota belongs to the selected provider accounts, not to a conversation's
/// historical runtime. Route members sharing an account show it once.
struct ComposerAccountQuota: Identifiable {
    enum Source {
        case provider(AIProvider, modelIDs: [String])
        case runtimeLogin(RuntimeInstance)
    }
    var source: Source
    let quota: AccountQuota?
    var id: String {
        switch source {
        case .provider(let provider, _): "provider:\(provider.id)"
        case .runtimeLogin(let runtime): "runtime:\(runtime.id)"
        }
    }
    var providerID: String? { if case .provider(let provider, _) = source { provider.id } else { nil } }
    var title: String {
        switch source {
        case .provider(let provider, _): provider.name
        case .runtimeLogin(let runtime): "\(runtime.name) · 登录额度"
        }
    }
    func limit(for windowID: String) -> UInt8? {
        guard case .provider(let provider, _) = source else { return nil }
        return provider.quotaLimits.first { $0.windowID == windowID }?.maxUsedPercent
    }
    var windows: [QuotaWindow] {
        guard case .provider(_, let modelIDs) = source else { return quota?.windows ?? [] }
        return (quota?.windows ?? []).filter { window in modelIDs.contains(where: window.applies) }
    }

    static func providerAccounts(for selection: ModelSelection?, gateway: GatewayConfig, quotas: [String: AccountQuota]) -> [Self] {
        guard case .managed(let key) = selection else { return [] }
        let keys = gateway.routes.first { $0.recordKey == key }?.members ?? [key]
        var accounts: [Self] = []
        for key in keys {
            guard let provider = gateway.providers.first(where: { $0.models.contains { $0.recordKey == key } }),
                  provider.composition.quotaSource != .none, !provider.authentication.awaitsSignIn,
                  let model = provider.models.first(where: { $0.recordKey == key }) else { continue }
            if let index = accounts.firstIndex(where: { $0.providerID == provider.id }),
               case .provider(_, var modelIDs) = accounts[index].source {
                if !modelIDs.contains(model.providerModelID) { modelIDs.append(model.providerModelID) }
                accounts[index].source = .provider(provider, modelIDs: modelIDs)
            } else {
                accounts.append(Self(source: .provider(provider, modelIDs: [model.providerModelID]), quota: quotas[provider.id]))
            }
        }
        return accounts
    }
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
