import Charts
import SwiftUI
import VeluneBindings

/// A series retained in quota history: a window's used share, or an amount.
enum QuotaMetric: Hashable, Sendable {
    case window(id: String, modelIDs: [String]?)
    case balance(id: String, unit: String)
    var title: String {
        switch self {
        case .window(let id, let models): return QuotaWindowObservation.title(id) + (models.map { "（仅 " + $0.joined(separator: "、") + "）" } ?? "")
        case .balance(let id, let unit): return QuotaBalanceObservation.title(id: id, unit: unit)
        }
    }
}
/// One successful observation the core retained.
struct QuotaHistorySample: Sendable, Equatable {
    var observedAt: Date
    var available: Bool?
    var values: [QuotaMetric: Double]
}

private enum QuotaHistoryRange: String, CaseIterable, Identifiable {
    case day, week, month
    var id: String { rawValue }
    var label: String { switch self { case .day: "24 小时"; case .week: "7 天"; case .month: "30 天" } }
    var duration: TimeInterval { switch self { case .day: 86_400; case .week: 7 * 86_400; case .month: 30 * 86_400 } }
}

/// The observations of one time bin; a bin reads as its latest observation.
private struct QuotaBin: Identifiable {
    var start: Date
    var end: Date
    var latest: Double
    var latestAt: Date
    var low: Double
    var high: Double
    var count: Int
    var id: Date { start }
}

/// A provider's retained quota over time. Only observations Velune made are
/// shown; time without one stays empty instead of carrying a value forward.
struct QuotaHistorySheet: View {
    @ObservedObject var store: AppStore
    let providerID: String
    @Environment(\.dismiss) private var dismiss
    @State private var range: QuotaHistoryRange = .day
    @State private var observations: [QuotaHistorySample]?
    @State private var failure: String?
    @State private var metric: QuotaMetric?
    @State private var selectedDate: Date?
    @State private var confirmingClear = false
    private static let binCount = 96
    private var provider: ProviderSummary? { store.providers.first { $0.id == providerID } }
    private var quota: QuotaObservation? { store.providerQuotas[providerID] }
    private var remaining: Bool { store.usageDisplay == .remaining }
    /// Current series first, in the order the provider reports them, then ones only history has.
    private var metrics: [QuotaMetric] {
        var all: [QuotaMetric] = (quota?.windows ?? []).map { .window(id: $0.id, modelIDs: $0.modelIds) } + (quota?.balances ?? []).map { .balance(id: $0.id, unit: $0.unit) }
        for observation in observations ?? [] {
            for key in observation.values.keys.sorted(by: { $0.title < $1.title }) where !all.contains(key) { all.append(key) }
        }
        return all
    }
    private var current: QuotaMetric? { metric.flatMap { metrics.contains($0) ? $0 : nil } ?? metrics.first }
    private var isWindow: Bool { if case .window = current { return true }; return false }
    /// A window's value as the user reads usage; an amount as reported.
    private func shown(_ value: Double) -> Double { isWindow && remaining ? 100 - min(max(value, 0), 100) : value }
    private var bins: [QuotaBin] {
        guard let current, let observations else { return [] }
        let end = Date(), width = range.duration / Double(Self.binCount), start = end.addingTimeInterval(-range.duration)
        var bins: [Int: QuotaBin] = [:]
        for observation in observations where observation.observedAt >= start {
            guard let raw = observation.values[current] else { continue }
            let value = shown(raw)
            let index = min(Self.binCount - 1, Int(observation.observedAt.timeIntervalSince(start) / width))
            let binStart = start.addingTimeInterval(Double(index) * width)
            if var bin = bins[index] {
                if observation.observedAt >= bin.latestAt { bin.latest = value; bin.latestAt = observation.observedAt }
                bin.low = min(bin.low, value); bin.high = max(bin.high, value); bin.count += 1
                bins[index] = bin
            } else {
                bins[index] = QuotaBin(start: binStart, end: binStart.addingTimeInterval(width), latest: value, latestAt: observation.observedAt, low: value, high: value, count: 1)
            }
        }
        return bins.values.sorted { $0.start < $1.start }
    }
    private var unitLabel: String {
        switch current {
        case .window: return remaining ? "剩余" : "已用"
        case .balance(_, let unit): return unit
        case nil: return ""
        }
    }
    private func format(_ value: Double) -> String {
        switch current {
        case .balance(_, let unit): return value.formatted(.currency(code: unit).presentation(.narrow))
        default: return "\(unitLabel) \(Int(value.rounded()))%"
        }
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("额度历史").font(.headline)
                if let provider { Text(provider.name).foregroundStyle(.secondary) }
                Spacer()
            }
            HStack {
                Picker("项目", selection: Binding(get: { current }, set: { metric = $0 })) {
                    ForEach(metrics, id: \.self) { Text($0.title).tag(QuotaMetric?.some($0)) }
                }.disabled(metrics.isEmpty).frame(maxWidth: 320)
                Spacer()
                Picker("范围", selection: $range) { ForEach(QuotaHistoryRange.allCases) { Text($0.label).tag($0) } }.pickerStyle(.segmented).labelsHidden().fixedSize()
            }
            content.frame(maxWidth: .infinity, minHeight: 220, maxHeight: .infinity)
            summary
            Text("只记录 Velune 读取到的额度：查看额度或经网关使用这个提供商时才会读取。空白时段表示没有观测，不代表额度没有变化。保留 30 天；更换认证后从新的记录开始。").formFooterStyle()
            HStack {
                Button("清除历史…", role: .destructive) { confirmingClear = true }.disabled(observations?.isEmpty ?? true)
                Spacer()
                Button("完成") { dismiss() }.keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 620, height: 480)
        .task(id: range) { load() }
        .onChange(of: quota?.observedAt) { _, _ in load() }
        .confirmationDialog("清除这个提供商的额度历史？", isPresented: $confirmingClear) {
            Button("清除历史", role: .destructive) { store.clearQuotaHistory(providerID) { load() } }
        } message: { Text("只删除保留的观测记录；当前额度、认证和使用上限不受影响。") }
    }
    @ViewBuilder private var content: some View {
        if let failure {
            VStack(spacing: 8) {
                Text("额度历史无法读取：\(failure)").foregroundStyle(.secondary)
                Button("重试") { load() }
            }.frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if observations == nil {
            ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if current == nil {
            Text(quota?.freshness == .unsupported ? "这个提供商不报告额度。" : "还没有记录到额度。").foregroundStyle(.secondary).frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if bins.isEmpty {
            Text("所选时间范围内没有观测。").foregroundStyle(.secondary).frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
            chart
        }
    }
    private var chart: some View {
        let width = range.duration / Double(Self.binCount)
        let selected = selectedDate.flatMap { date in bins.first { $0.start <= date && date < $0.end } }
        return Chart {
            ForEach(bins) { bin in
                RectangleMark(xStart: .value("开始", bin.start.addingTimeInterval(width * 0.15)), xEnd: .value("结束", bin.end.addingTimeInterval(-width * 0.15)), yStart: .value(unitLabel, 0), yEnd: .value(unitLabel, bin.latest))
                    .foregroundStyle(selected?.id == bin.id ? Color.accentColor : Color.accentColor.opacity(0.65))
            }
        }
        .chartYScale(domain: isWindow ? 0...100 : 0...max(1, (bins.map(\.high).max() ?? 1) * 1.1))
        .chartXScale(domain: Date().addingTimeInterval(-range.duration)...Date(), range: .plotDimension(endPadding: 28))
        .chartXAxis {
            AxisMarks(values: .automatic(desiredCount: range == .day ? 6 : 7)) { _ in
                AxisGridLine(); AxisTick()
                AxisValueLabel(format: range == .day ? .dateTime.hour() : .dateTime.month(.defaultDigits).day())
            }
        }
        .chartXSelection(value: $selectedDate)
        .chartYAxisLabel(isWindow ? (remaining ? "剩余 %" : "已用 %") : unitLabel)
        .overlay(alignment: .topLeading) {
            if let selected {
                VStack(alignment: .leading, spacing: 2) {
                    Text(format(selected.latest)).monospacedDigit()
                    Text("观测于 " + selected.latestAt.formatted(date: .abbreviated, time: .shortened)).font(.caption).foregroundStyle(.secondary)
                    if selected.count > 1 { Text("此区间 \(selected.count) 次观测，\(format(selected.low)) 至 \(format(selected.high))").font(.caption).foregroundStyle(.secondary) }
                }.padding(6).background(.regularMaterial, in: RoundedRectangle(cornerRadius: 6))
            }
        }
        .accessibilityLabel("额度历史图")
    }
    @ViewBuilder private var summary: some View {
        let points = bins
        if let last = points.last {
            let extreme = isWindow && !remaining ? points.map(\.high).max() : points.map(\.low).min()
            HStack(spacing: 18) {
                LabeledContent("最近一次", value: "\(format(last.latest)) · \(last.latestAt.formatted(.relative(presentation: .named)))")
                if let extreme { LabeledContent(isWindow && !remaining ? "观测到的最高" : "观测到的最低", value: format(extreme)) }
                if case .window(let id, let models) = current, let reset = quota?.windows.first(where: { $0.id == id && $0.modelIds == models })?.resetsAt, reset > Date() {
                    LabeledContent("下次重置", value: reset.formatted(.relative(presentation: .named)))
                }
            }.font(.callout).monospacedDigit()
        }
    }
    private func load() {
        failure = nil
        store.readQuotaHistory(providerID, since: Date().addingTimeInterval(-range.duration)) { result in
            switch result {
            case .success(let values): observations = values
            case .failure(let error): failure = error.localizedDescription
            }
        }
    }
}
