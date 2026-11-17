import SwiftUI
import Charts
import VeluneBindings

/// The provider's lifetime and date-labelled report has no declared timezone.
struct ProviderAccountProfileContent: View {
    let profile: ProviderAccountProfile
    var body: some View {
        Group {
            LabeledContent("累计 Token", value: profile.lifetimeTokens.map { TokenCount.format($0) } ?? "未报告")
            if let asOf = profile.statsAsOf { LabeledContent("官方统计截至", value: asOf) }
            if let generated = profile.generatedAt { LabeledContent("官方报表生成", value: ProviderReportFormatting.timestamp(generated)) }
            if let daily = profile.daily, !daily.isEmpty {
                ImmediateDisclosureGroup("按日期用量（\(daily.count) 天）") {
                    ProviderAccountDailyContent(daily: daily).padding(.top, 8)
                }
            } else { Text(profile.daily == nil ? "未报告按日期用量" : "此报告暂无按日期用量记录").foregroundStyle(.secondary) }
        }
    }
}

struct ProviderAccountDailyContent: View {
    let daily: [ProviderAccountDailyUsage]
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Chart(Array(daily.enumerated()), id: \.offset) { _, day in
                BarMark(x: .value("来源日期", day.date), y: .value("Token", Double(day.tokens)))
            }
            .chartYAxis {
                AxisMarks { value in
                    AxisGridLine(); AxisTick()
                    if let number = value.as(Double.self) { AxisValueLabel(TokenCount.rate(number)) }
                }
            }
            .frame(height: 160)
            Text("日期沿用提供商标签，来源未声明时区。").uiFont(.caption).foregroundStyle(.secondary)
            ForEach(Array(daily.enumerated()), id: \.offset) { _, day in
                LabeledContent(day.date, value: TokenCount.format(day.tokens))
            }
        }
    }
}
