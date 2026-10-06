import SwiftUI
import Charts
import VeluneBindings

struct AnalyticsButton: View {
    @Environment(\.openWindow) private var openWindow
    var body: some View {
        Button { openWindow(id: "analytics") } label: { Label("分析", systemImage: "chart.bar.xaxis") }
            .help("查看 Token 消耗与有效输出速率")
    }
}

struct AnalyticsMenuItem: View {
    @Environment(\.openWindow) private var openWindow
    var body: some View { Button("分析…") { openWindow(id: "analytics") } }
}

struct AnalyticsView: View {
    @ObservedObject var store: AppStore
    @Environment(\.scenePhase) private var scenePhase
    @State private var period = AnalyticsPeriod.sevenDays
    @State private var range = AnalyticsPeriod.sevenDays.timeRange()
    @State private var chartMetric = AnalyticsChartMetric.tokens
    @State private var chartSelection: Date?
    @State private var breakdownMode = AnalyticsBreakdownMode.provider
    @State private var selectedBreakdownID: String?
    @State private var scope: AnalyticsScope?
    @State private var showsRequests = false
    @State private var selectedRequestID: String?
    @State private var requestLimit: UInt32 = 100
    private var selectedRequest: BindingAnalyticsRequest? { store.analyticsReport?.requests.first { $0.requestId == selectedRequestID } }
    private var breakdownRows: [AnalyticsBreakdownRow] { (breakdownMode == .provider ? store.analyticsReport?.providers : store.analyticsReport?.models)?.map(AnalyticsBreakdownRow.init) ?? [] }
    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text(range.caption).help(range.description)
                Spacer()
                if let updated = store.analyticsUpdatedAt { Text("更新于 \(updated.formatted(date: .omitted, time: .shortened))") }
            }.font(.caption).foregroundStyle(.secondary).padding(.horizontal, 20).padding(.vertical, 10)
            if store.analyticsIsLoading { ProgressView("正在读取分析记录…").frame(maxWidth: .infinity, maxHeight: .infinity) }
            else if store.analyticsReadFailed {
                ContentUnavailableView { Label("无法读取分析记录", systemImage: "exclamationmark.triangle") } description: { Text("可在问题窗口查看详情，然后重试。") } actions: { Button("重试") { load() }; ProblemsButton(store: store) }
            } else if let report = store.analyticsReport, report.overview.requestCount > 0 {
                if showsRequests { requestList(report) }
                else { overview(report) }
            } else {
                ContentUnavailableView("暂无分析记录", systemImage: "chart.bar.xaxis", description: Text("此时间范围没有记录。分析从启用此功能后经 Velune LLM 网关发出的请求开始，不读取 Agent 运行时历史。"))
            }
            if let report = store.analyticsReport {
                Divider()
                HStack {
                    Text(report.storageWarning == nil ? "来源：Velune LLM 网关新请求的上游用量报告，不包含运行时历史。" : "统计记录可能不完整，请在问题窗口查看详情。")
                    Spacer()
                }.font(.caption).foregroundStyle(.secondary).padding(12)
            }
        }
        .navigationTitle(showsRequests ? scope?.name ?? "请求记录" : "分析")
        .toolbar {
            if showsRequests { ToolbarItem(placement: .navigation) { Button("概览", systemImage: "chevron.backward") { showsRequests = false; scope = nil; selectedRequestID = nil; load() } } }
            ToolbarItem { Picker("时间范围", selection: $period) { ForEach(AnalyticsPeriod.allCases) { Text($0.title).tag($0) } }.frame(width: 135) }
            ToolbarItem { Button("刷新", systemImage: "arrow.clockwise") { refresh() }.disabled(store.analyticsIsLoading) }
            ToolbarItem { ProblemsButton(store: store) }
        }
        .inspector(isPresented: Binding(get: { selectedRequest != nil }, set: { if !$0 { selectedRequestID = nil } })) {
            Group { if let selectedRequest { AnalyticsRequestDetail(request: selectedRequest) } }.inspectorColumnWidth(min: 280, ideal: 320, max: 400)
        }
        .task { refresh() }
        .onChange(of: breakdownMode) { _, _ in selectedBreakdownID = nil }
        .onChange(of: period) { _, _ in selectedRequestID = nil; chartSelection = nil; refresh() }
        .onChange(of: scenePhase) { _, value in if value == .active && !store.analyticsIsLoading { refresh() } }
        .onDisappear { store.cancelAnalyticsRead() }
    }
    private func refresh() { range = period.timeRange(); load() }
    private func load() {
        store.loadAnalytics(BindingAnalyticsQuery(fromMs: range.boundaries.first!, toMs: range.boundaries.last!, bucketBoundariesMs: range.boundaries, providerId: scope?.providerID, modelRecordKey: scope?.modelRecordKey, requestLimit: requestLimit))
    }
    private func showRequests(_ row: BindingAnalyticsBreakdown? = nil) {
        scope = row.map { AnalyticsScope(name: $0.name, providerID: $0.providerId, modelRecordKey: $0.modelRecordKey) }
        selectedRequestID = nil; requestLimit = 100; showsRequests = true; load()
    }
    private func overview(_ report: BindingAnalyticsReport) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                HStack(alignment: .top, spacing: 24) {
                    metric("已报告总量", value: AnalyticsFormatting.tokens(report.overview.totalTokens), coverage: coverage(report.overview.totalReportedCount, report.overview.requestCount)).help(AnalyticsFormatting.totalDefinition)
                    metric("输入 Token", value: AnalyticsFormatting.tokens(report.overview.inputTokens), coverage: coverage(report.overview.inputReportedCount, report.overview.requestCount))
                    metric("输出 Token", value: AnalyticsFormatting.tokens(report.overview.outputTokens), coverage: coverage(report.overview.outputReportedCount, report.overview.requestCount))
                    metric("有效输出速率", value: AnalyticsFormatting.speed(report.overview.outputTokensPerSecond), coverage: "\(report.overview.eligibleSpeedCount) / \(report.overview.requestCount) 次请求可计算")
                        .help(AnalyticsFormatting.speedDefinition)
                }
                Text(requestSummary(report.overview)).font(.callout).foregroundStyle(.secondary)
                Divider()
                trend(report)
                Divider()
                HStack {
                    Text("分解").font(.headline)
                    Picker("分解方式", selection: $breakdownMode) { ForEach(AnalyticsBreakdownMode.allCases) { Text($0.title).tag($0) } }.pickerStyle(.segmented).frame(width: 190)
                    Spacer()
                    Button("查看全部请求…") { showRequests() }
                }
                Table(breakdownRows, selection: $selectedBreakdownID) {
                    TableColumn(breakdownMode.title) { row in Text(row.value.name).lineLimit(1).help(row.value.name) }
                    TableColumn("请求") { row in Text(row.value.totals.requestCount, format: .number).monospacedDigit() }.width(70)
                    TableColumn("已报告总量") { row in Text(AnalyticsFormatting.tokens(row.value.totals.totalTokens)).monospacedDigit().help("\(coverage(row.value.totals.totalReportedCount, row.value.totals.requestCount))\n\(AnalyticsFormatting.totalDefinition)") }.width(125)
                    TableColumn("有效输出速率") { row in Text(AnalyticsFormatting.speed(row.value.totals.outputTokensPerSecond)).monospacedDigit() }.width(135)
                }.frame(minHeight: 190, idealHeight: 230)
                .contextMenu(forSelectionType: String.self) { ids in
                    if let id = ids.first, let row = breakdownRows.first(where: { $0.id == id }) { Button("查看请求…") { showRequests(row.value) } }
                } primaryAction: { ids in if let id = ids.first, let row = breakdownRows.first(where: { $0.id == id }) { showRequests(row.value) } }
                HStack { Spacer(); Button("查看请求…") { if let row = breakdownRows.first(where: { $0.id == selectedBreakdownID }) { showRequests(row.value) } }.disabled(selectedBreakdownID == nil) }
            }.padding(20)
        }
    }
    private func metric(_ title: String, value: String, coverage: String) -> some View {
        VStack(alignment: .leading, spacing: 6) { Text(title).font(.callout).foregroundStyle(.secondary); Text(value).font(.title2).monospacedDigit().lineLimit(1).minimumScaleFactor(0.7).help(value); Text(coverage).font(.caption).foregroundStyle(.secondary) }.frame(maxWidth: .infinity, alignment: .leading)
    }
    private func requestSummary(_ totals: BindingAnalyticsTotals) -> String {
        var parts = ["\(totals.requestCount) 次上游请求", "\(totals.completedCount) 次完成", "\(totals.failedCount) 次失败", "\(totals.cancelledCount) 次取消"]
        if totals.incompleteCount > 0 { parts.append("\(totals.incompleteCount) 次未完成") }
        if totals.rejectedCount > 0 { parts.append("\(totals.rejectedCount) 次拒绝") }
        return parts.joined(separator: " · ")
    }
    private func coverage(_ known: UInt64, _ total: UInt64) -> String { "\(known) / \(total) 次请求报告" }
    private func trend(_ report: BindingAnalyticsReport) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text("每日趋势").font(.headline)
                Spacer()
                Picker("趋势指标", selection: $chartMetric) { ForEach(AnalyticsChartMetric.allCases) { Text($0.title).tag($0) } }.pickerStyle(.segmented).frame(width: 210)
            }
            if hasChartData(report) {
            Chart(report.trend.map(AnalyticsBucketRow.init)) { row in
                if chartMetric == .tokens, let tokens = row.value.totals.totalTokens {
                    BarMark(x: .value("日期", Date(timeIntervalSince1970: Double(row.value.fromMs) / 1_000)), y: .value("已报告 Token", Double(tokens))).foregroundStyle(.tint)
                } else if chartMetric == .tokens && row.value.totals.requestCount == 0 {
                    BarMark(x: .value("日期", Date(timeIntervalSince1970: Double(row.value.fromMs) / 1_000)), y: .value("已报告 Token", 0)).foregroundStyle(.tint)
                } else if chartMetric == .speed, let speed = row.value.totals.outputTokensPerSecond {
                    PointMark(x: .value("日期", Date(timeIntervalSince1970: Double(row.value.fromMs) / 1_000)), y: .value("有效输出速率", speed)).foregroundStyle(.tint)
                }
            }
            .chartXScale(domain: range.start...range.end)
            .chartXSelection(value: $chartSelection)
            .chartXAxis { AxisMarks { value in AxisGridLine(); AxisTick(); if let date = value.as(Date.self) { AxisValueLabel(range.dayLabel(date)) } } }
            .frame(height: 180)
            } else { ContentUnavailableView(chartMetric == .tokens ? "暂无已报告的 Token 用量" : "暂无可计算的有效输出速率", systemImage: "chart.bar.xaxis").frame(height: 180) }
            if let chartSelection, let bucket = report.trend.first(where: { Double($0.fromMs) / 1_000 <= chartSelection.timeIntervalSince1970 && Double($0.toMs) / 1_000 > chartSelection.timeIntervalSince1970 }) {
                Text("\(range.dayLabel(Date(timeIntervalSince1970: Double(bucket.fromMs) / 1_000))) · \(bucket.totals.requestCount) 次请求 · 已报告 Token：\(AnalyticsFormatting.tokens(bucket.totals.totalTokens)) · 有效输出速率：\(AnalyticsFormatting.speed(bucket.totals.outputTokensPerSecond)) · \(coverage(bucket.totals.totalReportedCount, bucket.totals.requestCount))").font(.caption).foregroundStyle(.secondary)
            } else { Text("总量仅含输入、输出均已报告的请求。缺少报告的日期不补零；无请求日期为零。速率包含等待时间。").font(.caption).foregroundStyle(.secondary) }
        }
    }
    private func hasChartData(_ report: BindingAnalyticsReport) -> Bool {
        report.trend.contains { bucket in chartMetric == .tokens ? bucket.totals.totalTokens != nil || bucket.totals.requestCount == 0 : bucket.totals.outputTokensPerSecond != nil }
    }
    private func requestList(_ report: BindingAnalyticsReport) -> some View {
        VStack(spacing: 0) {
            Table(report.requests.map(AnalyticsRequestRow.init), selection: $selectedRequestID) {
                TableColumn("时间") { row in Text(Date(timeIntervalSince1970: Double(row.value.startedAtMs) / 1_000), format: .dateTime.month(.abbreviated).day().hour().minute()).monospacedDigit() }.width(125)
                TableColumn("提供商 / 模型") { row in VStack(alignment: .leading, spacing: 3) { Text(row.value.providerName); Text(row.value.providerModelId).font(.caption).foregroundStyle(.secondary) }.lineLimit(1) }
                TableColumn("结果") { row in Text(AnalyticsFormatting.outcome(row.value.outcome)) }.width(75)
                TableColumn("输出 Token") { row in Text(AnalyticsFormatting.tokens(row.value.outputTokens)).monospacedDigit() }.width(100)
                TableColumn("有效输出速率") { row in Text(AnalyticsFormatting.speed(row.value.outputTokensPerSecond)).monospacedDigit() }.width(135)
            }
            HStack {
                Text("显示最近 \(report.requests.count) / \(report.overview.requestCount) 条请求记录").font(.caption).foregroundStyle(.secondary)
                Spacer()
                if report.overview.requestCount > UInt64(report.requests.count) && requestLimit < 500 { Button("加载更多") { requestLimit = min(500, requestLimit + 100); load() } }
                else if report.overview.requestCount > UInt64(report.requests.count) { Text("最多展示最近 500 条").font(.caption).foregroundStyle(.secondary) }
            }.padding(12)
        }
    }
}

private struct AnalyticsRequestDetail: View {
    let request: BindingAnalyticsRequest
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                Text("请求详情").font(.headline)
                Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 10) {
                    row("结果", AnalyticsFormatting.outcome(request.outcome))
                    row("提供商", request.providerName)
                    row("模型 ID", request.providerModelId)
                    row("协议", AnalyticsFormatting.protocolName(request.protocol))
                    row("开始", Date(timeIntervalSince1970: Double(request.startedAtMs) / 1_000).formatted())
                    row("请求耗时", AnalyticsFormatting.duration(request.elapsedMs))
                    row("首输出延迟", AnalyticsFormatting.duration(request.firstOutputMs))
                    row("有效输出速率", AnalyticsFormatting.speed(request.outputTokensPerSecond))
                    row("输入 Token", AnalyticsFormatting.tokens(request.inputTokens))
                    row("输出 Token", AnalyticsFormatting.tokens(request.outputTokens))
                    row("推理 Token", AnalyticsFormatting.tokens(request.reasoningOutputTokens))
                }
                ImmediateDisclosureGroup {
                    Text(AnalyticsFormatting.totalDefinition).font(.caption).foregroundStyle(.secondary)
                    Text(AnalyticsFormatting.speedDefinition).font(.caption).foregroundStyle(.secondary)
                    Text("首输出延迟从上游派发到首次正文、推理或工具内容；非流式响应未知。推理 Token 属于输出的子集，不重复计入总量。").font(.caption).foregroundStyle(.secondary)
                    Text("请求耗时涵盖完成接收、失败或取消；有效输出速率仅使用可确认的业务终态耗时。— 表示上游未报告或无法计算。").font(.caption).foregroundStyle(.secondary)
                } label: { Text("指标说明") }
                ImmediateDisclosureGroup {
                    Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 10) {
                        row("普通输入", AnalyticsFormatting.tokens(request.uncachedInputTokens))
                        row("缓存输入", AnalyticsFormatting.tokens(request.cachedInputTokens))
                        row("缓存读取", AnalyticsFormatting.tokens(request.cacheReadInputTokens))
                        row("缓存创建", AnalyticsFormatting.tokens(request.cacheCreationInputTokens))
                    }
                    Text("缓存字段按上游协议保留，不再次加到已归一的输入总量中。").font(.caption).foregroundStyle(.secondary)
                } label: { Text("缓存用量") }
                ImmediateDisclosureGroup {
                    Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 10) {
                        row("请求编号", request.requestId)
                        row("HTTP 状态", request.status.map { String($0) } ?? "—")
                        row("业务终态耗时", AnalyticsFormatting.duration(request.terminalElapsedMs))
                        row("用量报告", request.usageReported ? request.usageComplete ? "完整" : "部分" : "未报告")
                    }
                } label: { Text("请求信息") }
            }.padding(20).textSelection(.enabled)
        }
    }
    private func row(_ title: String, _ value: String) -> some View { GridRow { Text(title).foregroundStyle(.secondary); Text(value).frame(maxWidth: .infinity, alignment: .leading) } }
}

private enum AnalyticsFormatting {
    static let totalDefinition = "总量单位为 Token，仅合计输入和输出均已报告的请求。输入、输出列分别合计各自已报告部分，覆盖可能不同，不能直接相加比较。"
    static let speedDefinition = "有效输出速率 = 已报告输出 Token ÷ 上游派发至业务终态的请求耗时，包含等待；汇总按可计算请求加权，不是单次速率的平均值。"
    static func tokens(_ value: UInt64?) -> String { value.map { $0.formatted(.number) } ?? "—" }
    static func speed(_ value: Double?) -> String { guard let value, value.isFinite else { return "—" }; return value.formatted(.number.precision(.fractionLength(1))) + " Token/s" }
    static func duration(_ value: UInt64?) -> String { value.map { Duration.milliseconds(Double($0)).formatted(.units(allowed: [.minutes, .seconds], width: .abbreviated, fractionalPart: .show(length: 2))) } ?? "—" }
    static func outcome(_ value: BindingAnalyticsOutcome) -> String { switch value { case .completed: return "完成"; case .incomplete: return "未完成"; case .failed: return "失败"; case .cancelled: return "取消"; case .rejected: return "拒绝" } }
    static func protocolName(_ value: BindingAnalyticsProtocol) -> String { switch value { case .chatCompletions: return "OpenAI ChatCompletions"; case .responses: return "OpenAI Responses"; case .messages: return "Anthropic Messages" } }
}
private struct AnalyticsScope { var name: String; var providerID: String?; var modelRecordKey: String? }
private struct AnalyticsBreakdownRow: Identifiable { let value: BindingAnalyticsBreakdown; var id: String { value.key } }
private struct AnalyticsBucketRow: Identifiable { let value: BindingAnalyticsBucket; var id: Int64 { value.fromMs } }
private struct AnalyticsRequestRow: Identifiable { let value: BindingAnalyticsRequest; var id: String { value.requestId } }
private enum AnalyticsBreakdownMode: String, CaseIterable, Identifiable { case provider, model; var id: Self { self }; var title: String { self == .provider ? "提供商" : "模型" } }
private enum AnalyticsChartMetric: String, CaseIterable, Identifiable { case tokens, speed; var id: Self { self }; var title: String { self == .tokens ? "已报告总量" : "有效输出速率" } }
private enum AnalyticsPeriod: String, CaseIterable, Identifiable {
    case today, sevenDays, thirtyDays
    var id: Self { self }
    var title: String { switch self { case .today: return "今天"; case .sevenDays: return "最近 7 天"; case .thirtyDays: return "最近 30 天" } }
    var days: Int { switch self { case .today: return 1; case .sevenDays: return 7; case .thirtyDays: return 30 } }
    func timeRange(now: Date = Date(), calendar: Calendar = .current) -> AnalyticsTimeRange {
        let start = calendar.date(byAdding: .day, value: 1 - days, to: calendar.startOfDay(for: now))!
        var boundaries: [Int64] = []
        var cursor = start
        while cursor < now { boundaries.append(Int64(cursor.timeIntervalSince1970 * 1_000)); cursor = calendar.date(byAdding: .day, value: 1, to: cursor)! }
        boundaries.append(Int64(now.timeIntervalSince1970 * 1_000))
        return AnalyticsTimeRange(start: start, end: now, boundaries: boundaries, timeZone: calendar.timeZone)
    }
}
private struct AnalyticsTimeRange {
    var start: Date
    var end: Date
    var boundaries: [Int64]
    var timeZone: TimeZone
    var caption: String { "\(dayLabel(start))–\(dayLabel(end)) · \(timeZone.identifier)" }
    var description: String { "\(instantLabel(start)) 至 \(instantLabel(end))（不含结束时刻）" }
    private func instantLabel(_ date: Date) -> String { var style = Date.FormatStyle(date: .abbreviated, time: .shortened); style.timeZone = timeZone; return date.formatted(style) }
    func dayLabel(_ date: Date) -> String { var style = Date.FormatStyle(date: .abbreviated, time: .omitted); style.timeZone = timeZone; return date.formatted(style) }
}
