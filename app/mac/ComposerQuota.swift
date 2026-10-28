import Foundation
import VeluneBindings

/// Quota belongs to the selected provider accounts, not to a conversation's
/// historical runtime. Route members sharing an account show it once.
struct ComposerAccountQuota: Identifiable {
    let provider: AIProvider
    var modelIDs: [String]
    let quota: ProviderQuota?
    var id: String { provider.id }
    var windows: [QuotaWindow] {
        (quota?.windows ?? []).filter { window in modelIDs.contains(where: window.applies) }
    }

    static func accounts(for selection: ModelSelection?, gateway: GatewayConfig, quotas: [String: ProviderQuota]) -> [Self] {
        guard case .managed(let key) = selection else { return [] }
        let keys = gateway.routes.first { $0.recordKey == key }?.members ?? [key]
        var accounts: [Self] = []
        for key in keys {
            guard let provider = gateway.providers.first(where: { $0.models.contains { $0.recordKey == key } }),
                  provider.composition.quotaSource != .none, !provider.authentication.awaitsSignIn,
                  let model = provider.models.first(where: { $0.recordKey == key }) else { continue }
            if let index = accounts.firstIndex(where: { $0.id == provider.id }) {
                if !accounts[index].modelIDs.contains(model.providerModelID) { accounts[index].modelIDs.append(model.providerModelID) }
            } else {
                accounts.append(Self(provider: provider, modelIDs: [model.providerModelID], quota: quotas[provider.id]))
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
