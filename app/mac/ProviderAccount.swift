import SwiftUI
import VeluneBindings

/// Account balances and reset entitlements are snapshots, separate from quota gates.
struct ProviderReportReadIdentity: Hashable {
    let store: ObjectIdentifier
    let gatewayID: String
    let revision: UInt64
    let providerID: String?
    @MainActor init(store: DeviceStore, providerID: String? = nil) {
        self.store = ObjectIdentifier(store)
        gatewayID = store.gateway.id
        revision = store.providerReportRevision
        self.providerID = providerID
    }
}

struct ProviderAccountSection: View {
    @ObservedObject var store: DeviceStore
    let providerID: String
    var body: some View {
        ProviderAccountReader(store: store, providerID: providerID)
            .id(ProviderReportReadIdentity(store: store, providerID: providerID))
    }
}

private struct ProviderAccountReader: View {
    @ObservedObject var store: DeviceStore
    let providerID: String
    @State private var status: ProviderAccountStatus?
    @State private var problem: String?
    @State private var reading = false
    @State private var refreshID = 0

    var body: some View {
        Section {
            if let status {
                ProviderAccountSnapshot(status: status)
            } else if reading {
                ProgressView("正在读取账户…")
            } else if problem == nil {
                Text("尚未取得账户快照").foregroundStyle(.secondary)
            }
            if let problem { Label(problem, systemImage: "exclamationmark.triangle").textSelection(.enabled) }
        } header: {
            HStack {
                Text("订阅账户")
                Spacer()
                Button(reading ? "正在读取…" : "刷新") { refreshID += 1 }.disabled(reading).controlSize(.small)
            }
        } footer: {
            Text("credits 按提供商原始单位显示；重置权益是可用次数，均不计入 Token 或费用，也不影响使用上限。")
                .formFooterStyle()
        }
        .task(id: "\(providerID):\(refreshID)") { await read(refresh: refreshID > 0) }
    }
    private func read(refresh: Bool) async {
        reading = true
        problem = nil
        defer { if !Task.isCancelled { reading = false } }
        // Refresh once. Subsequent bounded reads only follow the core's cached job.
        for attempt in 0..<24 {
            let result: Result<ProviderAccountStatus, Error> = await withCheckedContinuation { continuation in
                store.readProviderAccount(providerID, refresh: refresh && attempt == 0) { continuation.resume(returning: $0) }
            }
            guard !Task.isCancelled else { return }
            switch result {
            case .success(let value):
                status = value
                if !value.refreshing { return }
            case .failure(let error): problem = error.localizedDescription; return
            }
            if attempt < 23 { try? await Task.sleep(for: .seconds(2)) }
        }
        problem = "账户查询仍在后台进行；稍后可再次刷新。"
    }
}

struct ProviderAccountSnapshot: View {
    let status: ProviderAccountStatus
    var body: some View {
        Group {
            LabeledContent("状态", value: ProviderReportFormatting.accountState(status.state))
            if status.stale { Text("上次账户快照已过期，以下保留上次结果。").foregroundStyle(.secondary) }
            if let fetchedAt = status.fetchedAt { LabeledContent("本地抓取", value: ProviderReportFormatting.timestamp(fetchedAt)) }
            ProviderAccountAmounts(credits: status.credits, resets: status.bankedResets)
            if let observed = status.creditsObservedAt { LabeledContent("余额抓取", value: ProviderReportFormatting.timestamp(observed)) }
            if let observed = status.resetCountObservedAt { LabeledContent("权益次数抓取", value: ProviderReportFormatting.timestamp(observed)) }
            if let observed = status.resetGrantsObservedAt { LabeledContent("权益明细抓取", value: ProviderReportFormatting.timestamp(observed)) }
            if !status.staleSources.isEmpty {
                Text("已过期：" + status.staleSources.map(ProviderReportFormatting.accountFailureSource).joined(separator: "、"))
                    .uiFont(.caption).foregroundStyle(.secondary)
            }
            ForEach(Array(status.failures.enumerated()), id: \.offset) { _, failure in
                Label("\(ProviderReportFormatting.accountFailureSource(failure.source))：\(failure.message)", systemImage: "exclamationmark.triangle").textSelection(.enabled)
            }
        }
    }
}

struct ProviderAccountAmounts: View {
    let credits: ProviderAccountCredits?
    let resets: ProviderAccountResets?
    var body: some View {
        Group {
            LabeledContent("credits 余额", value: credits?.unlimited == true ? "不限量（提供商报告）" : credits?.balance ?? "未报告")
            if let hasCredits = credits?.hasCredits {
                LabeledContent("credits 可用", value: hasCredits ? "是" : "否")
            }
            LabeledContent("可用重置权益", value: resets?.availableCount.map { "\($0.formatted()) 次" } ?? "未报告")
            if let grants = resets?.grants {
                if grants.isEmpty { Text("当前没有重置权益明细").foregroundStyle(.secondary) }
                else {
                    ImmediateDisclosureGroup("重置权益明细（\(grants.count)）") {
                        VStack(alignment: .leading, spacing: 12) {
                            ForEach(Array(grants.enumerated()), id: \.offset) { _, grant in
                                ProviderAccountResetGrantContent(grant: grant)
                            }
                        }.padding(.top, 8)
                    }
                }
            } else { Text("重置权益明细未取得").uiFont(.caption).foregroundStyle(.secondary) }
        }.textSelection(.enabled)
    }
}

struct ProviderAccountResetGrantContent: View {
    let grant: ProviderAccountResetGrant
    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(grant.title ?? grant.resetType)
            LabeledContent("提供商状态", value: grant.status)
            if let expires = grant.expiresAt { LabeledContent("到期", value: ProviderReportFormatting.timestamp(expires)) }
            if let granted = grant.grantedAt { LabeledContent("发放", value: ProviderReportFormatting.timestamp(granted)) }
            if let description = grant.description { Text(description).foregroundStyle(.secondary) }
        }
    }
}

enum ProviderReportFormatting {
    static func timestamp(_ value: String) -> String {
        guard let date = ISO8601DateFormatter().date(from: value) else { return value }
        return date.formatted(date: .abbreviated, time: .standard)
    }
    static func milliseconds(_ value: Int64) -> String {
        Date(timeIntervalSince1970: Double(value) / 1_000).formatted(date: .abbreviated, time: .standard)
    }
    static func accountState(_ state: ProviderAccountState) -> String {
        switch state {
        case .unsupported: return "此提供商不支持账户报告"
        case .unauthenticated: return "需要登录账户"
        case .refreshing: return "正在查询"
        case .expired: return "认证已过期，请重新登录"
        case .available: return "已取得账户报告"
        case .partialFailure: return "部分数据未取得"
        case .failed: return "查询失败，可刷新重试"
        }
    }
    static func accountFailureSource(_ source: ProviderAccountSource) -> String {
        switch source {
        case .usage: return "账户余额"
        case .resets: return "重置权益"
        case .profile: return "账户用量"
        }
    }
}
