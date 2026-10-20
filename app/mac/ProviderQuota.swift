import SwiftUI
import VeluneBindings

/// One quota window's usage against the provider's ceiling for it.
struct QuotaWindowRow: View {
    let window: QuotaWindow
    let limit: UInt8?
    /// An expired observation is shown for reference only.
    let fresh: Bool
    let display: BindingUsageDisplay
    var showsModels = false
    private var reached: Bool { fresh && limit.map { window.usedPercent >= Double($0) } ?? false }
    private var used: Double { min(max(window.usedPercent, 0), 100) }
    /// The share the row shows, used or remaining as the user chose.
    private var shown: Double { display == .remaining ? 100 - used : used }
    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack(spacing: 8) {
                Text("\(QuotaWindow.name(window.id))窗口").frame(minWidth: 70, alignment: .leading)
                ProgressView(value: shown, total: 100).tint(reached ? Color.orange : nil)
                Text(UsageFormat.label(Int(shown.rounded()), display)).monospacedDigit().foregroundStyle(fresh ? .primary : .secondary).frame(width: 64, alignment: .trailing)
            }
            if !detail.isEmpty { Text(detail).font(.caption).foregroundStyle(reached ? .orange : .secondary) }
        }
    }
    private var detail: String {
        var parts: [String] = []
        if let limit { parts.append(UsageFormat.ceiling(limit, display, reached: reached)) }
        if let reset = window.resetsAt, reset > Date() { parts.append("\(reset.formatted(.relative(presentation: .named)))重置") }
        if showsModels, let models = window.modelIDs { parts.append("仅 " + models.joined(separator: "、")) }
        return parts.joined(separator: " · ")
    }
}

/// Usage as the user reads it: the used share, or what remains of it.
enum UsageFormat {
    static func label(_ usedOrRemaining: Int, _ display: BindingUsageDisplay) -> String {
        display == .remaining ? "剩余 \(usedOrRemaining)%" : "已用 \(usedOrRemaining)%"
    }
    /// A used-percent ceiling; read as remaining, it is the share kept back.
    static func ceiling(_ limit: UInt8, _ display: BindingUsageDisplay, reached: Bool) -> String {
        switch (display, reached) {
        case (.used, false): "上限 \(limit)%"
        case (.used, true): "已达上限 \(limit)%，暂停新请求"
        case (.remaining, false): "保留 \(100 - Int(limit))%"
        case (.remaining, true): "剩余已不足保留的 \(100 - Int(limit))%，暂停新请求"
        }
    }
}

/// When the quota was observed, or why it is missing.
struct QuotaStatusText: View {
    let quota: ProviderQuota?
    var body: some View {
        Text(text).font(.caption).foregroundStyle(.secondary)
    }
    private var text: String {
        guard let quota else { return "正在读取额度…" }
        if quota.refreshing && quota.freshness != .fresh { return "正在读取额度…" }
        let observed = quota.observedAt.map { $0.formatted(.relative(presentation: .named)) }
        switch quota.freshness {
        case .fresh: return observed.map { "额度于\($0)更新" } ?? "额度已更新"
        case .stale: return "额度已过期" + (observed.map { "（\($0)观测）" } ?? "") + "，再次使用或刷新时更新"
        case .unknown: return quota.problem ?? "尚未取得额度"
        case .unsupported: return "此提供商不报告额度"
        }
    }
}

/// What using a subscription provider means; shown wherever it is chosen or used.
struct SubscriptionDisclosureNote: View {
    let disclosure: SubscriptionDisclosure
    var body: some View {
        Label {
            VStack(alignment: .leading, spacing: 2) {
                Text(disclosure.title).font(.callout)
                Text(disclosure.detail).font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            }
        } icon: { Image(systemName: "exclamationmark.triangle.fill").symbolRenderingMode(.multicolor) }
    }
}

extension View {
    /// Explanatory text under a form section, read from its leading edge.
    func formFooterStyle() -> some View {
        font(.caption).foregroundStyle(.secondary).multilineTextAlignment(.leading).fixedSize(horizontal: false, vertical: true).frame(maxWidth: .infinity, alignment: .leading)
    }
    /// Keeps the providers' quota current while the view shows: reads it at
    /// once, again every two seconds while a sample is under way or nothing
    /// has been read, and otherwise every half minute. Reading lets the core
    /// sample an expired observation; it never polls an idle provider itself.
    func readsQuota(of providerIDs: [String], from store: AppStore) -> some View {
        task(id: providerIDs) {
            guard !providerIDs.isEmpty else { return }
            var lastRead = Date.distantPast
            while !Task.isCancelled {
                let waiting = providerIDs.contains { store.providerQuotas[$0]?.refreshing ?? true }
                if waiting || Date().timeIntervalSince(lastRead) >= 30 { store.readProviderQuotas(providerIDs); lastRead = Date() }
                try? await Task.sleep(for: .seconds(2))
            }
        }
    }
}

/// The provider editor's quota pane: the latest observation and the
/// used-percent ceiling of each window, which every route and direct use of
/// the provider is held to.
struct ProviderQuotaForm: View {
    @ObservedObject var store: AppStore
    /// `nil` until the provider is saved.
    let providerID: String?
    @Binding var limits: [QuotaLimit]
    private static let presets: [UInt8] = [50, 60, 70, 80, 90, 95, 100]
    private var quota: ProviderQuota? { providerID.flatMap { store.providerQuotas[$0] } }
    /// Quota is read with the login, so a provider awaiting one has none to show.
    private var signedOut: Bool { providerID.flatMap { id in store.providers.first { $0.id == id } }?.authentication.awaitsSignIn ?? true }
    /// Reported windows first, then ceilings for windows not reported now.
    private var windowIDs: [String] {
        var ids: [String] = []
        for id in (quota?.windows.map(\.id) ?? []) + limits.map(\.windowID) where !ids.contains(id) { ids.append(id) }
        return ids
    }
    var body: some View {
        Form {
            Section {
                if providerID == nil {
                    Text("保存提供商并配置认证后读取额度。").foregroundStyle(.secondary)
                } else if signedOut {
                    Text("配置认证后读取额度。").foregroundStyle(.secondary)
                } else {
                    if let quota, quota.available == false {
                        Label("提供商表示此账户当前不可用，暂停新请求。", systemImage: "pause.circle").foregroundStyle(.orange)
                    }
                    ForEach(Array((quota?.windows ?? []).enumerated()), id: \.offset) { _, window in
                        QuotaWindowRow(window: window, limit: limit(window.id), fresh: quota?.freshness == .fresh, display: store.usageDisplay, showsModels: true)
                    }
                    QuotaStatusText(quota: quota)
                }
            } header: {
                HStack {
                    Text("额度")
                    Spacer()
                    if let providerID, !signedOut {
                        Button("刷新") { store.readProviderQuotas([providerID], refresh: true) }.disabled(quota?.refreshing == true).controlSize(.small)
                    }
                }
            }
            Section {
                if windowIDs.isEmpty {
                    Text("取得额度后可为各窗口设置上限。").foregroundStyle(.secondary)
                } else {
                    ForEach(windowIDs, id: \.self) { id in
                        Picker(selection: limitBinding(id)) {
                            Text("不限制").tag(UInt8?.none)
                            ForEach(choices(id), id: \.self) { Text(store.usageDisplay == .remaining ? "保留 \(100 - Int($0))%" : "已用 \($0)%").tag(UInt8?.some($0)) }
                        } label: {
                            Text("\(QuotaWindow.name(id))窗口")
                            if quota?.windows.contains(where: { $0.id == id }) != true { Text("当前未报告此窗口") }
                        }
                    }
                }
            } header: {
                Text("使用上限")
            } footer: {
                Text((store.usageDisplay == .remaining ? "任一窗口的剩余额度低于保留比例时，暂停向此提供商发送新请求，直到剩余回到保留比例以上" : "任一窗口的已用比例达到上限时，暂停向此提供商发送新请求，直到额度降回上限以下") + "；对所有模型路由和直接选择它的模型都生效。设了上限却取不到最新额度时也会暂停。上限依据定期观测，已开始的请求和其他工具的使用仍可能越过它。")
                    .formFooterStyle()
            }
        }
        .readsQuota(of: signedOut ? [] : providerID.map { [$0] } ?? [], from: store)
    }
    private func limit(_ id: String) -> UInt8? { limits.first { $0.windowID == id }?.maxUsedPercent }
    private func choices(_ id: String) -> [UInt8] {
        guard let current = limit(id), !Self.presets.contains(current) else { return Self.presets }
        return (Self.presets + [current]).sorted()
    }
    private func limitBinding(_ id: String) -> Binding<UInt8?> {
        Binding(get: { limit(id) }, set: { value in
            if let value {
                if let index = limits.firstIndex(where: { $0.windowID == id }) { limits[index].maxUsedPercent = value } else { limits.append(QuotaLimit(windowID: id, maxUsedPercent: value)) }
            } else { limits.removeAll { $0.windowID == id } }
        })
    }
}
