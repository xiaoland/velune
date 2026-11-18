import SwiftUI
import VeluneBindings

struct AnalyticsUsageBreakdown: View {
    let local: AnalyticsReport?
    let provider: ProviderAnalyticsReport?
    let fromMs: Int64
    let toMs: Int64
    let byModel: Bool
    let showRequests: (AnalyticsBreakdown) -> Void

    @State private var selection: UsageComparisonRow.ID?
    @State private var showingProviderDetail = false

    var body: some View {
        let rows = UsageComparisonRow.make(local: local, provider: provider, byModel: byModel,
                                           window: UsageComparisonWindow(fromMs: fromMs, toMs: toMs))
        let selected = rows.first { $0.id == selection }
        Group {
            if rows.isEmpty {
                ContentUnavailableView(byModel ? "暂无模型用量" : "暂无提供商用量", systemImage: "chart.bar.xaxis")
            } else {
                VStack(spacing: 0) {
                    comparisonTable(rows)
                    Divider()
                    HStack {
                        Spacer()
                        Button("查看提供商明细…") { showingProviderDetail = true }
                            .disabled(selected?.observation == nil)
                            .popover(isPresented: $showingProviderDetail, arrowEdge: .top) {
                                if let selected, let observation = selected.observation {
                                    UsageComparisonDetail(title: selected.title, observation: observation)
                                }
                            }
                        Button("查看请求…") { if let value = selected?.local { showRequests(value) } }
                            .disabled(selected?.local == nil)
                    }.padding(.horizontal, 20).padding(.vertical, 9)
                }
            }
        }
        .onChange(of: rows.map(\.id)) { _, ids in
            if let selection, !ids.contains(selection) { self.selection = nil }
        }
        .onChange(of: byModel) { _, _ in selection = nil }
        .onChange(of: selected?.observation != nil) { _, available in
            if !available { showingProviderDetail = false }
        }
    }

    private func comparisonTable(_ rows: [UsageComparisonRow]) -> some View {
        Table(rows, selection: $selection) {
            TableColumn(byModel ? "模型" : "提供商") { row in
                cell(row.title, subtitle: row.subtitle).help(row.title)
            }.width(min: 125, ideal: 140)
            TableColumn("网关请求") { row in
                Text(row.local?.totals.requestCount.formatted() ?? "—").monospacedDigit()
            }.width(68)
            TableColumn("网关 Token") { row in
                gatewayTokenCell(row.local?.totals)
            }.width(min: 95, ideal: 100)
            TableColumn("网关输出速率") { row in
                let speed = row.local?.totals.outputTokensPerSecond.flatMap { $0.isFinite ? TokenCount.rate($0) + " Token/s" : nil }
                Text(speed ?? "—").lineLimit(1).monospacedDigit()
                    .help("已报告输出 Token ÷ 上游派发至业务终态的请求耗时，包含等待时间。")
            }.width(103)
            TableColumn("提供商用量") { row in
                if let observation = row.observation {
                    cell(observation.summary, subtitle: observation.tableCoverage).monospacedDigit()
                        .help([observation.summary, observation.tableCoverage, observation.explanation]
                            .compactMap { $0 }.joined(separator: "\n"))
                } else {
                    Text("—").foregroundStyle(.secondary)
                }
            }.width(min: 145, ideal: 150)
        }
        .contextMenu(forSelectionType: UsageComparisonRow.ID.self) { ids in
            if let row = rows.first(where: { ids.contains($0.id) }) {
                if let value = row.local { Button("查看请求…") { showRequests(value) } }
                if row.observation != nil {
                    Button("查看提供商明细…") { selection = row.id; showingProviderDetail = true }
                }
            }
        } primaryAction: { ids in
            if let row = rows.first(where: { ids.contains($0.id) }) {
                if let value = row.local { showRequests(value) }
                else if row.observation != nil { selection = row.id; showingProviderDetail = true }
            }
        }
    }

    private func cell(_ title: String, subtitle: String?) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title)
            if let subtitle { Text(subtitle).uiFont(.caption).foregroundStyle(.secondary) }
        }.lineLimit(1).truncationMode(.middle)
    }

    private func gatewayTokenCell(_ totals: AnalyticsTotals?) -> some View {
        let input = totals?.inputTokens.map { TokenCount.format($0) } ?? "—"
        let output = totals?.outputTokens.map { TokenCount.format($0) } ?? "—"
        let detail = totals.map { totals in
            let count = totals.requestCount
            return "输入 \(totals.inputTokens?.formatted() ?? "未报告")，\(totals.inputReportedCount) / \(count) 次报告\n输出 \(totals.outputTokens?.formatted() ?? "未报告")，\(totals.outputReportedCount) / \(count) 次报告"
        }
        return Text(totals == nil ? "—" : "入 \(input) / 出 \(output)").lineLimit(1).monospacedDigit()
            .help(detail ?? "暂无网关用量")
    }
}

private struct UsageComparisonRow: Identifiable {
    enum ID: Hashable { case local(String), provider(String, model: Data?) }

    let id: ID
    let title: String
    let subtitle: String?
    let local: AnalyticsBreakdown?
    var observation: UsageComparisonObservation?

    static func make(local: AnalyticsReport?, provider: ProviderAnalyticsReport?, byModel: Bool,
                     window: UsageComparisonWindow) -> [Self] {
        let localProviders = local?.providers ?? []
        let providerNames = Dictionary(localProviders.compactMap { value in
            value.providerId.map { ($0, value.name) }
        }, uniquingKeysWith: { first, _ in first })
        let localRows = byModel ? (local?.models ?? []) : localProviders
        var rows = localRows.map { value in
            Self(id: .local(value.key), title: value.name,
                 subtitle: byModel ? value.providerId.flatMap { providerNames[$0] } : nil,
                 local: value, observation: nil)
        }
        let sources = provider?.sources ?? []
        let comparisonIDs = provider?.comparisonSourceIds.map { Set($0) }
        // The local journal keys by provider ID alone. A multi-gateway ID, or a
        // source shared by several configurations, cannot be assigned to one local row.
        var gatewaysByProvider: [String: Set<String>] = [:]
        for source in sources {
            for id in source.providerIds { gatewaysByProvider[id, default: []].formUnion(source.gatewayIds) }
        }
        let groupedSources = Dictionary(grouping: sources) { source in
            if !byModel, let profile = source.profileIdentityKey { return "profile:\(profile)" }
            if let identity = source.identityKey { return "identity:\(source.kind):\(identity)" }
            return "source:\(source.id)"
        }
        for key in groupedSources.keys.sorted() {
            guard let associated = groupedSources[key] else { continue }
            let candidates = associated.filter { comparisonIDs?.contains($0.id) ?? true }
            let ids = Set(associated.flatMap(\.providerIds))
            let gateways = Set(associated.flatMap(\.gatewayIds))
            let localID = ids.count == 1 && gateways.count == 1
                ? ids.first.flatMap { gatewaysByProvider[$0]?.count == 1 ? $0 : nil } : nil
            let modelKeys: [Data?]
            if byModel {
                let modelIDs = Set(candidates.flatMap { ($0.modelUsage ?? []).map { Data($0.model.utf8) } })
                modelKeys = modelIDs.sorted { $0.lexicographicallyPrecedes($1) }.map(Optional.some)
            } else { modelKeys = [nil] }
            for modelKey in modelKeys {
                let model = modelKey.map { String(decoding: $0, as: UTF8.self) }
                let observations = candidates.compactMap { source in
                    UsageComparisonObservation(source: source, model: model, window: window)
                }
                guard let observation = observations.sorted(by: { $0.precedes($1) }).first else { continue }
                let matches = rows.indices.filter { index in
                    guard let localID, let value = rows[index].local, value.providerId == localID else { return false }
                    return model.map { $0.utf8.elementsEqual(value.name.utf8) } ?? true
                }
                if matches.count == 1, let match = matches.first, rows[match].observation == nil {
                    rows[match].observation = observation
                } else {
                    let names = Set(associated.flatMap(\.providerNames)).sorted().joined(separator: "、")
                    rows.append(Self(id: .provider(key, model: modelKey),
                                     title: model ?? (names.isEmpty ? observation.source.label : names),
                                     subtitle: model == nil || names.isEmpty ? observation.scope : names,
                                     local: nil, observation: observation))
                }
            }
        }
        return rows
    }
}

private struct UsageComparisonWindow {
    let fromMs: Int64
    let toMs: Int64
    let dateLabels: ClosedRange<String>?

    init(fromMs: Int64, toMs: Int64) {
        self.fromMs = fromMs
        self.toMs = toMs
        guard fromMs < toMs else { dateLabels = nil; return }
        let formatter = DateFormatter()
        formatter.calendar = Calendar(identifier: .gregorian)
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.timeZone = .current
        formatter.dateFormat = "yyyy-MM-dd"
        func label(_ value: Int64) -> String { formatter.string(from: Date(timeIntervalSince1970: Double(value) / 1_000)) }
        dateLabels = label(fromMs)...label(toMs - 1)
    }
}

private struct UsageComparisonObservation {
    let source: ProviderAnalyticsSource
    let bins: [UsageComparisonBin]
    let totals: [UsageComparisonMetric]
    let coverage: String
    let explanation: String?
    let area: ProviderAnalyticsArea

    var scope: String {
        ProviderReportFormatting.scope(source.scope, id: source.scopeId ?? (source.scope == .organization ? source.organizationId : nil))
    }
    var observedAt: Int64? {
        switch area {
        case .modelUsage: return source.modelUsageObservedAtMs
        case .profile: return source.profileObservedAtMs
        default: return source.observedAtMs
        }
    }
    var isStale: Bool { source.staleAreas.contains(area) || (source.stale && source.staleAreas.isEmpty) }
    var summary: String {
        let input = totals.first { $0.label == "输入 Token" }
        let output = totals.first { $0.label == "输出 Token" }
        if input != nil || output != nil {
            return "入 \(input?.value ?? "—") / 出 \(output?.value ?? "—")"
        }
        if let metric = totals.first { return "\(metric.value) \(metric.label)" }
        return bins.isEmpty ? "—" : "查看分桶"
    }

    var tableCoverage: String { isStale ? "上次数据，\(coverage)" : coverage }

    init?(source: ProviderAnalyticsSource, model: String?, window: UsageComparisonWindow) {
        self.source = source
        if let model {
            area = .modelUsage
            let available = (source.modelUsage ?? []).filter { $0.model.utf8.elementsEqual(model.utf8) }
            let labels = window.dateLabels
            let selected = available.map(UsageComparisonBin.init).filter { bin in
                if let range = bin.range { return range.lowerBound < window.toMs && range.upperBound > window.fromMs }
                return bin.date.map { labels?.contains($0) == true } ?? true
            }
            guard !selected.isEmpty else { return nil }
            let dated = selected.filter { $0.range == nil && $0.date != nil }
            let bounded = selected.filter { bin in
                bin.range.map { $0.lowerBound >= window.fromMs && $0.upperBound <= window.toMs } ?? false
            }.sorted { ($0.range?.lowerBound ?? 0) < ($1.range?.lowerBound ?? 0) }
            let disjoint = !zip(bounded, bounded.dropFirst()).contains { pair in
                guard let first = pair.0.range, let second = pair.1.range else { return false }
                return first.overlaps(second)
            }
            let uniqueDays = Set(dated.compactMap(\.date)).count == dated.count
            let compatible: [UsageComparisonBin]
            if dated.isEmpty && disjoint { compatible = bounded }
            else if selected.allSatisfy({ $0.range == nil }) && uniqueDays { compatible = dated }
            else { compatible = [] }
            bins = selected.sorted { ($0.range?.lowerBound ?? .min, $0.date ?? "") < ($1.range?.lowerBound ?? .min, $1.date ?? "") }
            totals = UsageComparisonMetric.sum(compatible)
            if compatible.isEmpty { coverage = "来源分桶" }
            else if dated.isEmpty { coverage = "\(compatible.count) 个完整 UTC 分桶" }
            else { coverage = "\(compatible.count) 个来源日期" }
            var notes: [String] = []
            if !dated.isEmpty { notes.append("日期沿用提供商标签，来源未声明时区。") }
            if compatible.count < selected.count { notes.append("只有范围内可合计的完整分桶计入小计。") }
            if selected.contains(where: { $0.relativeUsage != nil }) { notes.append("相对用量按原始分桶显示。") }
            explanation = notes.isEmpty ? nil : notes.joined()
        } else if source.kind == .chatgptSubscription {
            area = .profile
            let labels = window.dateLabels
            let daily = (source.profile?.daily ?? []).filter { labels?.contains($0.date) == true }
            bins = daily.compactMap { day in
                UInt64(exactly: day.tokens).map { UsageComparisonBin(date: day.date, tokens: $0) }
            }
            totals = Set(daily.map(\.date)).count == daily.count ? UsageComparisonMetric.sum(bins) : []
            coverage = daily.isEmpty ? "所选日期无报告" : "\(Set(daily.map(\.date)).count) 个来源日期"
            explanation = "日期沿用提供商标签，来源未声明时区。"
        } else {
            area = .organizationUsage
            bins = []
            if let usage = source.usage {
                totals = [.tokens("输入 Token", usage.inputTokens), .tokens("输出 Token", usage.outputTokens),
                          .integer("请求", usage.requests)] + (usage.cachedInputTokens.map { [.tokens("缓存读取 Token", $0)] } ?? [])
                          + (usage.cacheCreationInputTokens.map { [.tokens("缓存创建 Token", $0)] } ?? [])
            } else { totals = [] }
            if let start = source.rangeStartAtMs, let end = source.rangeEndAtMs, start < end {
                coverage = "组织本月（UTC）"
                explanation = ProviderReportFormatting.utcRange(start, end)
            } else {
                coverage = "组织用量未报告"
                explanation = nil
            }
        }
    }

    func precedes(_ other: Self) -> Bool {
        let hasValues = !totals.isEmpty || !bins.isEmpty
        let otherHasValues = !other.totals.isEmpty || !other.bins.isEmpty
        if hasValues != otherHasValues { return hasValues }
        if isStale != other.isStale { return !isStale }
        if observedAt != other.observedAt { return (observedAt ?? .min) > (other.observedAt ?? .min) }
        return source.id < other.source.id
    }
}

private struct UsageComparisonBin {
    let date: String?
    let range: Range<Int64>?
    let inputTokens: UInt64?
    let uncachedInputTokens: UInt64?
    let cachedInputTokens: UInt64?
    let outputTokens: UInt64?
    let requests: UInt64?
    let turns: UInt64?
    let credits: Double?
    let relativeUsage: Double?
    let tokens: UInt64?

    init(_ value: ProviderAnalyticsModelUsage) {
        if let start = value.fromMs, let end = value.toMs, start < end { range = start..<end }
        else { range = nil }
        date = value.date
        inputTokens = value.inputTokens
        uncachedInputTokens = value.uncachedInputTokens
        cachedInputTokens = value.cachedInputTokens
        outputTokens = value.outputTokens
        requests = value.requests
        turns = value.turns
        credits = value.credits
        relativeUsage = value.relativeUsage
        tokens = nil
    }

    init(date: String, tokens: UInt64) {
        self.date = date
        self.tokens = tokens
        range = nil
        inputTokens = nil
        uncachedInputTokens = nil
        cachedInputTokens = nil
        outputTokens = nil
        requests = nil
        turns = nil
        credits = nil
        relativeUsage = nil
    }

    var title: String {
        if let range { return ProviderReportFormatting.utcRange(range.lowerBound, range.upperBound) }
        return date ?? "来源未报告日期"
    }
}

private struct UsageComparisonMetric {
    let label: String
    let value: String

    static func tokens(_ label: String, _ amount: UInt64) -> Self { Self(label: label, value: TokenCount.format(amount)) }
    static func integer(_ label: String, _ amount: UInt64) -> Self { Self(label: label, value: amount.formatted()) }
    static func decimal(_ label: String, _ amount: Double) -> Self { Self(label: label, value: amount.formatted(.number.precision(.significantDigits(1...15)))) }

    static func sum(_ bins: [UsageComparisonBin]) -> [Self] {
        guard !bins.isEmpty else { return [] }
        func integerTotal(_ keyPath: KeyPath<UsageComparisonBin, UInt64?>) -> UInt64? {
            var total: UInt64 = 0
            for bin in bins {
                guard let value = bin[keyPath: keyPath] else { return nil }
                let (next, overflow) = total.addingReportingOverflow(value)
                guard !overflow else { return nil }
                total = next
            }
            return total
        }
        let tokenFields: [(String, KeyPath<UsageComparisonBin, UInt64?>)] = [
            ("输入 Token", \.inputTokens), ("输出 Token", \.outputTokens), ("Token", \.tokens),
            ("未缓存输入 Token", \.uncachedInputTokens), ("缓存输入 Token", \.cachedInputTokens)
        ]
        let countFields: [(String, KeyPath<UsageComparisonBin, UInt64?>)] = [("请求", \.requests), ("轮次", \.turns)]
        var metrics = tokenFields.compactMap { label, path in integerTotal(path).map { tokens(label, $0) } }
        metrics += countFields.compactMap { label, path in integerTotal(path).map { integer(label, $0) } }
        let credits = bins.compactMap(\.credits)
        if credits.count == bins.count {
            let total = credits.reduce(0, +)
            if total.isFinite { metrics.append(decimal("credits", total)) }
        }
        return metrics
    }
}

private struct UsageComparisonDetail: View {
    let title: String
    let observation: UsageComparisonObservation

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                Text(title).uiFont(.headline)
                VStack(alignment: .leading, spacing: 5) {
                    LabeledContent("来源", value: ProviderReportFormatting.sourceKind(observation.source.kind))
                    LabeledContent("统计范围", value: observation.scope)
                    LabeledContent("提供商配置", value: observation.source.providerNames.joined(separator: "、"))
                    if let observed = observation.observedAt {
                        LabeledContent("数据抓取", value: ProviderReportFormatting.milliseconds(observed))
                    }
                    if observation.isStale {
                        Label("显示上次取得的数据", systemImage: "exclamationmark.triangle")
                    }
                    if let explanation = observation.explanation { Text(explanation).foregroundStyle(.secondary) }
                }.uiFont(.callout)
                if !observation.totals.isEmpty {
                    Divider()
                    Text(observation.coverage).uiFont(.subheadline)
                    metrics(observation.totals)
                }
                if !observation.bins.isEmpty {
                    Divider()
                    ForEach(Array(observation.bins.enumerated()), id: \.offset) { _, bin in
                        VStack(alignment: .leading, spacing: 4) {
                            Text(bin.title).uiFont(.subheadline)
                            metrics(UsageComparisonMetric.sum([bin]))
                            if let relative = bin.relativeUsage, relative.isFinite {
                                LabeledContent("相对用量", value: UsageComparisonMetric.decimal("", relative).value)
                            }
                        }
                    }
                } else if observation.totals.isEmpty {
                    Text(ProviderReportFormatting.analyticsState(observation.source.state, kind: observation.source.kind)).foregroundStyle(.secondary)
                }
            }.padding(16).textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
        }.frame(width: 405, height: 365)
    }

    private func metrics(_ metrics: [UsageComparisonMetric]) -> some View {
        ForEach(metrics, id: \.label) { metric in LabeledContent(metric.label, value: metric.value).monospacedDigit() }
    }
}
