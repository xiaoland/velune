import SwiftUI
import Charts
import VeluneBindings

/// Shown beside the gateway's usage on the same overview. Scope and dates
/// belong to the provider observations.
struct AnalyticsProviderUsageSummary: View {
    let report: ProviderAnalyticsReport?
    let from: Date
    let to: Date
    let reading: Bool
    let problem: String?
    let showSources: () -> Void
    let showModels: () -> Void

    private var days: [ProviderAnalyticsDailyTotal] {
        let range = sourceDate(from)...sourceDate(to)
        return report?.chatgptProfileTotals.daily.filter { range.contains($0.date) } ?? []
    }
    private var dailyTokens: UInt64? {
        guard !days.isEmpty else { return nil }
        var total: UInt64 = 0
        for day in days {
            let addition = total.addingReportingOverflow(day.tokens)
            guard !addition.overflow else { return nil }
            total = addition.partialValue
        }
        return total
    }
    private var hasModelData: Bool {
        report?.sources.contains { $0.modelUsage?.isEmpty == false } == true
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 14) {
                Text("提供商用量").uiFont(.subheadline.weight(.semibold))
                if reading { ProgressView().controlSize(.small).accessibilityLabel("正在读取提供商用量") }
                if report?.sources.isEmpty == false || problem != nil {
                    Button("来源与范围", systemImage: "info.circle", action: showSources)
                        .buttonStyle(.borderless).uiFont(.caption)
                }
            }
            if let dailyTokens {
                VStack(alignment: .leading, spacing: 4) {
                    Text("ChatGPT 订阅").uiFont(.callout)
                    Text("\(TokenCount.format(dailyTokens)) Token").uiFont(.title3).monospacedDigit()
                    if let first = days.first?.date, let last = days.last?.date {
                        Text(first == last ? "\(first)，提供商日期" : "\(first) 至 \(last)，已报告 \(days.count) 天")
                            .uiFont(.caption).foregroundStyle(.secondary)
                    }
                }
            }
            if let report {
                ForEach(Array(report.organizationTotals.enumerated()), id: \.offset) { _, total in
                    VStack(alignment: .leading, spacing: 3) {
                        Text(total.services.map { $0 == .openAi ? "OpenAI Platform" : "Anthropic API" }.joined(separator: "、"))
                            .uiFont(.callout)
                        HStack(spacing: 12) {
                            if let input = total.inputTokens { Text("输入 \(TokenCount.format(input))").monospacedDigit() }
                            if let output = total.outputTokens { Text("输出 \(TokenCount.format(output))").monospacedDigit() }
                        }.uiFont(.caption)
                        Text("本月，UTC；查询至 \(ProviderReportFormatting.utcTimestamp(total.rangeEndMaxAtMs))")
                            .uiFont(.caption).foregroundStyle(.secondary)
                    }
                }
            }
            if hasModelData {
                Button("查看模型用量", action: showModels).buttonStyle(.link).uiFont(.caption)
            } else if dailyTokens == nil, report?.organizationTotals.isEmpty != false, !reading, problem == nil {
                Text(report?.sources.isEmpty == false ? "当前范围暂无已报告用量" : "暂无提供商用量")
                    .uiFont(.callout).foregroundStyle(.secondary)
            }
            if let problem { Label(problem, systemImage: "exclamationmark.triangle").uiFont(.caption).foregroundStyle(.secondary) }
            else if report?.sources.contains(where: \.stale) == true {
                Text("部分来源显示上次结果").uiFont(.caption).foregroundStyle(.secondary)
            }
        }.frame(minWidth: 240, maxWidth: .infinity, alignment: .leading)
    }

    private func sourceDate(_ date: Date) -> String {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = .current
        let parts = calendar.dateComponents([.year, .month, .day], from: date)
        return String(format: "%04d-%02d-%02d", parts.year ?? 0, parts.month ?? 0, parts.day ?? 0)
    }
}

struct ProviderAnalyticsContent: View {
    let report: ProviderAnalyticsReport?
    let reading: Bool
    let problem: String?
    let retry: () -> Void
    var body: some View {
        VStack(spacing: 0) {
            if let report {
                if reading || problem != nil {
                    HStack(alignment: .firstTextBaseline) {
                        if let problem { Text(problem); Spacer(); Button("重试", action: retry).disabled(reading) }
                        else { Text("正在查询，已有结果继续保留。"); Spacer() }
                    }.uiFont(.callout).foregroundStyle(.secondary).padding(.horizontal, 20).padding(.vertical, 10)
                }
                if report.sources.isEmpty {
                    ContentUnavailableView("暂无提供商报告", systemImage: "chart.bar.xaxis", description: Text("添加 ChatGPT 订阅，或在 API 提供商的“用量与账单”中配置组织查询。"))
                } else { ProviderAnalyticsReportContent(report: report) }
            } else if reading {
                ProgressView("正在读取提供商报告…").frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ContentUnavailableView {
                    Label("无法读取提供商报告", systemImage: "exclamationmark.triangle")
                } description: { Text(problem ?? "尚未取得报告。") } actions: { Button("重试", action: retry) }
            }
        }
    }
}

struct ProviderAnalyticsReportContent: View {
    let report: ProviderAnalyticsReport
    var body: some View {
        Form {
            Section {
                LabeledContent("本地读取", value: ProviderReportFormatting.milliseconds(report.observedAtMs))
                if report.sources.contains(where: \.stale) {
                    Text("部分来源数据已过期，汇总可能包含上次已知值。请在来源明细核对抓取时间。")
                        .foregroundStyle(.secondary)
                }
                Text("提供商报告可能包含其他客户端的用量，不与本地记录相加；此来源不提供逐请求明细或性能。")
                    .foregroundStyle(.secondary)
            }
            if report.sources.contains(where: { $0.kind == .chatgptSubscription }) {
                Section {
                    ProviderAnalyticsProfileTotalContent(total: report.chatgptProfileTotals)
                } header: { Text("ChatGPT 账户用量汇总") } footer: {
                    Text("按提供商账户档案身份去重；累计与每日分别统计已知部分。日期沿用来源标签，未声明时区，不与组织本月或本地记录合计。")
                        .formFooterStyle()
                }
            }
            Section {
                if report.organizationTotals.isEmpty {
                    Text("尚无可汇总的组织报表").foregroundStyle(.secondary)
                } else {
                    ForEach(Array(report.organizationTotals.enumerated()), id: \.offset) { _, total in
                        ProviderAnalyticsOrganizationContent(total: total)
                    }
                }
            } header: { Text("组织报告汇总（UTC）") } footer: {
                Text("汇总按统计范围去重，同范围的用量与费用分别选取可用报告，各指标不重复相加。同一组织优先使用全量报告；没有可用全量时，仅合计同一种互斥子范围。其余范围保留明细；费用按币种分别汇总。")
                    .formFooterStyle()
            }
            Section {
                ForEach(report.sources, id: \.id) { source in
                    ImmediateDisclosureGroup {
                        ProviderAnalyticsSourceDetail(source: source).padding(.top, 8)
                    } label: {
                        VStack(alignment: .leading, spacing: 4) {
                            Text(source.label).fixedSize(horizontal: false, vertical: true)
                            Text(ProviderReportFormatting.analyticsState(source.state, kind: source.kind) + (source.stale ? "，上次数据已过期" : ""))
                                .uiFont(.caption).foregroundStyle(.secondary)
                            if source.providerIds.count > 1 {
                                Text("\(source.providerIds.count) 个配置共用此来源，仅计一次").uiFont(.caption).foregroundStyle(.secondary)
                            }
                        }
                    }
                }
            } header: { Text("来源明细（\(report.sources.count)）") } footer: {
                Text("展开查看统计范围、官方截止时间与失败项。未报告表示未知，不视作零；累计用量与余额、重置权益分开。")
                    .formFooterStyle()
            }
        }.formStyle(.grouped)
    }
}

struct ProviderAnalyticsProfileTotalContent: View {
    let total: ProviderAnalyticsChatgptProfileTotal
    var body: some View {
        LabeledContent("已知累计 Token", value: total.lifetimeTokens.map { TokenCount.format($0) } ?? "未报告")
        if total.profileCount == 0 {
            Text("尚无可用于汇总的账户档案，请在来源明细查看状态。").uiFont(.caption).foregroundStyle(.secondary)
        } else {
            Text("累计覆盖 \(total.lifetimeSourceCount) / \(total.profileCount) 个账户档案；已取得 \(total.profileReportedCount) 个档案报告")
                .uiFont(.caption).foregroundStyle(.secondary)
            Text("日期用量覆盖 \(total.dailyReportedProfileCount) / \(total.profileCount) 个账户档案")
                .uiFont(.caption).foregroundStyle(.secondary)
        }
        if !total.daily.isEmpty {
            ImmediateDisclosureGroup("按日期的已知用量（\(total.daily.count) 天）") {
                VStack(alignment: .leading, spacing: 8) {
                    Chart(total.daily, id: \.date) { day in
                        BarMark(x: .value("来源日期", day.date), y: .value("Token", Double(day.tokens)))
                    }
                    .chartYAxis {
                        AxisMarks { value in
                            AxisGridLine(); AxisTick()
                            if let number = value.as(Double.self) { AxisValueLabel(TokenCount.rate(number)) }
                        }
                    }
                    .frame(height: 160)
                    ForEach(total.daily, id: \.date) { day in
                        LabeledContent(day.date) {
                            VStack(alignment: .trailing, spacing: 2) {
                                Text(TokenCount.format(day.tokens))
                                Text("\(day.sourceCount) 个档案报告").uiFont(.caption).foregroundStyle(.secondary)
                            }
                        }
                    }
                }.padding(.top, 8)
            }
        } else { Text(total.dailyReportedProfileCount == 0 ? "日期用量未报告" : "已报告档案暂无日期用量记录").foregroundStyle(.secondary) }
    }
}

struct ProviderAnalyticsOrganizationContent: View {
    let total: ProviderAnalyticsOrganizationTotal
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(total.services.map { $0 == .openAi ? "OpenAI Platform" : "Anthropic API" }.joined(separator: "、")).uiFont(.headline)
            if let end = total.rangeEndAtMs {
                LabeledContent("统计区间", value: ProviderReportFormatting.utcRange(total.rangeStartAtMs, end))
            } else {
                LabeledContent("统计起点", value: ProviderReportFormatting.utcTimestamp(total.rangeStartAtMs))
                LabeledContent("各来源截至", value: ProviderReportFormatting.utcRange(total.rangeEndMinAtMs, total.rangeEndMaxAtMs))
            }
            if total.rangeIsHeterogeneous {
                Text("各来源截至时间不同，以下为已报告小计。").uiFont(.callout).foregroundStyle(.secondary)
            }
            LabeledContent("组织", value: total.organizationIds.joined(separator: "、"))
            LabeledContent("汇总范围", value: ProviderReportFormatting.scope(total.scope, id: total.scopeIds.isEmpty ? nil : total.scopeIds.joined(separator: "、")))
            LabeledContent("输入 Token（含缓存）", value: total.inputTokens.map { TokenCount.format($0) } ?? "未报告")
            LabeledContent("输出 Token", value: total.outputTokens.map { TokenCount.format($0) } ?? "未报告")
            Text("用量覆盖 \(total.usageSourceCount) / \(total.sourceCount) 个去重范围").uiFont(.caption).foregroundStyle(.secondary)
            if total.costs.isEmpty {
                Text(total.costSourceCount == 0 ? "费用未报告" : "已报告范围暂无费用记录").foregroundStyle(.secondary)
            }
            ForEach(Array(total.costs.enumerated()), id: \.offset) { _, cost in
                LabeledContent("已知费用", value: ProviderReportFormatting.money(cost.amount, cost.currency))
            }
            Text("费用覆盖 \(total.costSourceCount) / \(total.sourceCount) 个去重范围").uiFont(.caption).foregroundStyle(.secondary)
        }.textSelection(.enabled).padding(.vertical, 4)
    }
}

struct ProviderAnalyticsSourceDetail: View {
    let source: ProviderAnalyticsSource
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            LabeledContent("提供商配置", value: source.providerNames.joined(separator: "、"))
            LabeledContent("来源", value: ProviderReportFormatting.sourceKind(source.kind))
            LabeledContent("统计范围", value: ProviderReportFormatting.scope(source.scope, id: source.scopeId ?? (source.scope == .organization ? source.organizationId : nil)))
            if source.scope != .organization, let organization = source.organizationId { LabeledContent("组织", value: organization) }
            if let start = source.rangeStartAtMs, let end = source.rangeEndAtMs {
                LabeledContent("统计区间", value: ProviderReportFormatting.utcRange(start, end))
            }
            if let observed = source.observedAtMs { LabeledContent("本地抓取", value: ProviderReportFormatting.milliseconds(observed)) }
            if let refreshed = source.dataRefreshedAtMs { LabeledContent("官方数据更新", value: ProviderReportFormatting.milliseconds(refreshed)) }
            if source.refreshing { Text("正在后台查询，以下保留已有结果。").foregroundStyle(.secondary) }
            if source.stale { Text("数据已过期，以下为上次取得的结果。").foregroundStyle(.secondary) }
            if !source.staleAreas.isEmpty {
                Text("已过期：" + source.staleAreas.map(ProviderReportFormatting.analyticsArea).joined(separator: "、"))
                    .uiFont(.caption).foregroundStyle(.secondary)
            }
            if source.kind == .chatgptSubscription {
                if let observed = source.profileObservedAtMs { LabeledContent("账户用量抓取", value: ProviderReportFormatting.milliseconds(observed)) }
                if let profile = source.profile { ProviderAccountProfileContent(profile: profile) }
                else { Text("账户用量未报告").foregroundStyle(.secondary) }
                Divider()
                ProviderAccountAmounts(credits: source.credits, resets: source.bankedResets)
                if let observed = source.creditsObservedAtMs { LabeledContent("余额抓取", value: ProviderReportFormatting.milliseconds(observed)) }
                if let observed = source.resetCountObservedAtMs { LabeledContent("权益次数抓取", value: ProviderReportFormatting.milliseconds(observed)) }
                if let observed = source.resetGrantsObservedAtMs { LabeledContent("权益明细抓取", value: ProviderReportFormatting.milliseconds(observed)) }
                Text("credits 和重置权益为账户快照，不计入历史消费。").uiFont(.caption).foregroundStyle(.secondary)
            } else {
                if let usage = source.usage {
                    LabeledContent("请求", value: usage.requests.formatted())
                    LabeledContent("输入 Token（含缓存）", value: TokenCount.format(usage.inputTokens))
                    LabeledContent("输出 Token", value: TokenCount.format(usage.outputTokens))
                    if let cached = usage.cachedInputTokens { LabeledContent("其中缓存读取", value: TokenCount.format(cached)) }
                    if let created = usage.cacheCreationInputTokens { LabeledContent("其中缓存创建", value: TokenCount.format(created)) }
                } else { Text("组织用量未报告").foregroundStyle(.secondary) }
                if let costs = source.costs {
                    if costs.isEmpty { Text("此范围暂无费用记录").foregroundStyle(.secondary) }
                    ForEach(Array(costs.enumerated()), id: \.offset) { _, cost in
                        LabeledContent("费用", value: ProviderReportFormatting.money(cost.amount, cost.currency))
                    }
                } else { Text("费用未报告").foregroundStyle(.secondary) }
            }
            ForEach(Array(source.failures.enumerated()), id: \.offset) { _, failure in
                Label("\(ProviderReportFormatting.analyticsArea(failure.area))：\(failure.message)", systemImage: "exclamationmark.triangle")
            }
        }.textSelection(.enabled)
    }
}

extension ProviderReportFormatting {
    static func money(_ amount: Double, _ currency: String) -> String {
        "\(currency) \(amount.formatted(.number.precision(.fractionLength(2...6))))"
    }
    static func utcTimestamp(_ value: Int64) -> String {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.timeZone = TimeZone(secondsFromGMT: 0)
        formatter.dateFormat = "yyyy-MM-dd HH:mm 'UTC'"
        return formatter.string(from: Date(timeIntervalSince1970: Double(value) / 1_000))
    }
    static func utcRange(_ start: Int64, _ end: Int64) -> String {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.timeZone = TimeZone(secondsFromGMT: 0)
        formatter.dateFormat = "yyyy-MM-dd HH:mm"
        func date(_ value: Int64) -> String { formatter.string(from: Date(timeIntervalSince1970: Double(value) / 1_000)) }
        return "\(date(start)) 至 \(date(end)) UTC"
    }
    static func sourceKind(_ kind: ProviderAnalyticsSourceKind) -> String {
        switch kind {
        case .chatgptSubscription: return "ChatGPT 订阅账户"
        case .openAiOrganization: return "OpenAI 组织 Usage / Costs"
        case .anthropicOrganization: return "Anthropic 组织 Usage / Costs"
        }
    }
    static func scope(_ scope: ProviderAnalyticsScope, id: String?) -> String {
        let name: String
        switch scope {
        case .subscriptionAccount: name = "订阅账户"
        case .organization: name = "组织全量"
        case .project: name = "Project"
        case .workspace: name = "Workspace"
        case .apiKey: name = "API key"
        }
        return id.map { "\(name)：\($0)" } ?? name
    }
    static func analyticsState(_ state: ProviderAnalyticsSourceState, kind: ProviderAnalyticsSourceKind) -> String {
        switch state {
        case .unsupported: return "此提供商不支持报告"
        case .unconfigured: return "尚未配置组织查询"
        case .unauthenticated: return "需要登录或配置查询认证"
        case .refreshing: return "正在查询"
        case .expired: return kind == .chatgptSubscription ? "认证已过期，请重新登录" : "上次报告已过期"
        case .available: return "已取得报告"
        case .partialFailure: return "部分数据未取得"
        case .failed: return "查询失败，可刷新重试"
        }
    }
    static func analyticsArea(_ area: ProviderAnalyticsArea) -> String {
        switch area {
        case .accountUsage: return "账户余额"
        case .bankedResets: return "重置权益"
        case .profile: return "账户用量"
        case .modelUsage: return "按模型用量"
        case .organizationUsage: return "组织用量"
        case .costs: return "费用"
        case .spendLimit: return "支出限额"
        case .rateLimits: return "速率上限"
        }
    }
}
