import SwiftUI
import Charts
import VeluneBindings

struct AnalyticsMenuItem: View {
    @Environment(\.openWindow) private var openWindow
    var body: some View { Button("分析") { openWindow(id: "analytics") } }
}

struct AnalyticsView: View {
    @ObservedObject var store: DeviceStore
    @Environment(\.scenePhase) private var scenePhase
    @State private var page = AnalyticsPage.overview
    @State private var period = AnalyticsPeriod.sevenDays
    @State private var range = AnalyticsPeriod.sevenDays.timeRange()
    @State private var chartMetric = AnalyticsChartMetric.tokens
    @State private var chartSelection: Date?
    @State private var breakdownMode = AnalyticsBreakdownMode.provider
    @State private var scope: AnalyticsScope?
    @State private var selectedRequestID: String?
    @State private var requestDetail: AnalyticsRequestRow?
    @State private var requestLimit: UInt32 = 100
    @State private var sources: AnalyticsProviderObservation?
    @State private var providerReadID = 0
    @State private var providerLoading = false
    @State private var providerError: String?
    @State private var showsProviderSources = false
    private var providerIdentity: ProviderReportReadIdentity { ProviderReportReadIdentity(store: store) }
    private var providerTask: AnalyticsProviderTask { AnalyticsProviderTask(identity: providerIdentity, refreshID: providerReadID) }
    private var providerReport: ProviderAnalyticsReport? { sources?.identity == providerIdentity ? sources?.report : nil }
    private var requestTableScope: AnalyticsRequestTableScope { AnalyticsRequestTableScope(providerID: store.analyticsLoadedQuery?.providerId, modelRecordKey: store.analyticsLoadedQuery?.modelRecordKey) }
    private var loadedRange: AnalyticsTimeRange? { store.analyticsLoadedQuery.map { AnalyticsTimeRange(start: Date(timeIntervalSince1970: Double($0.fromMs) / 1_000), end: Date(timeIntervalSince1970: Double($0.toMs) / 1_000), boundaries: $0.bucketBoundariesMs, timeZone: .current) } }
    private var loadedScopeName: String {
        guard let query = store.analyticsLoadedQuery else { return "全部请求" }
        if let model = query.modelRecordKey {
            let modelName = store.analyticsReport?.models.first { $0.modelRecordKey == model && $0.providerId == query.providerId }?.name ?? scope?.name ?? "所选模型"
            let providerName = store.analyticsReport?.providers.first { $0.providerId == query.providerId }?.name
            return [providerName, modelName].compactMap { $0 }.joined(separator: "，")
        }
        if let provider = query.providerId { return store.analyticsReport?.providers.first { $0.providerId == provider }?.name ?? scope?.name ?? "所选提供商" }
        return "全部请求"
    }
    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Picker("分析页面", selection: $page) { ForEach(AnalyticsPage.allCases) { Text($0.title).tag($0) } }.pickerStyle(.segmented).labelsHidden().frame(width: 360)
                Spacer()
            }.padding(.horizontal, 20).padding(.vertical, 12)
            if let report = store.analyticsReport { reportContext(report) }
            if let error = store.analyticsReadError {
                ReadFailureNotice(title: "无法读取网关记录", detail: error,
                                  retryDisabled: store.analyticsIsLoading, retry: load)
                    .padding(.horizontal, 8)
                Divider()
            }
            switch page {
            case .overview:
                if store.analyticsReport != nil || providerReport != nil || providerError != nil {
                    overview(store.analyticsReport)
                } else { gatewayPlaceholder }
            case .breakdown:
                if store.analyticsReport != nil || providerReport != nil {
                    breakdown(store.analyticsReport)
                } else { gatewayPlaceholder }
            case .requests:
                if let report = store.analyticsReport, !report.requests.isEmpty {
                    requestList(report)
                } else { gatewayPlaceholder }
            }
        }
        .navigationTitle("分析")
        .toolbar(id: "analytics") {
            ToolbarItem(id: "period") { Picker("时间范围", selection: $period) { ForEach(AnalyticsPeriod.allCases) { Text($0.title).tag($0) } }.labelsHidden().frame(width: 135).accessibilityLabel("时间范围") }
            ToolbarItem(id: "refresh") {
                HStack(spacing: 8) {
                    if store.analyticsIsLoading || providerLoading { ProgressView().controlSize(.small).accessibilityLabel("正在刷新分析") }
                    ReadRefreshButton(label: "刷新分析") { refresh(); providerReadID += 1 }
                        .disabled(store.analyticsIsLoading)
                }
            }
        }
        .sheet(item: $requestDetail) { row in AnalyticsRequestDetail(request: row.value) }
        .task { refresh() }
        .task(id: providerTask) { await readProviders(providerTask) }
        .onChange(of: page) { _, value in
            if value != .requests, scope != nil { scope = nil; load() }
        }
        .onChange(of: period) { _, _ in chartSelection = nil; refresh() }
        .onChange(of: store.analyticsUpdatedAt) { _, _ in
            if let selectedRequestID, store.analyticsReport?.requests.contains(where: { $0.requestId == selectedRequestID }) != true { self.selectedRequestID = nil }
        }
        .onChange(of: scenePhase) { _, value in if value == .active && !store.analyticsIsLoading { refresh() } }
        .onDisappear { store.cancelAnalyticsRead() }
    }
    @ViewBuilder private var gatewayPlaceholder: some View {
        if store.analyticsReadError != nil {
            Spacer(minLength: 0)
        } else if store.analyticsIsLoading || (page != .requests && providerLoading) {
            ProgressView("正在读取分析…").frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
            ContentUnavailableView("暂无网关请求", systemImage: "chart.bar.xaxis",
                description: Text("所示时间范围与筛选没有网关请求记录。"))
        }
    }

    private func readProviders(_ task: AnalyticsProviderTask) async {
        guard !store.isPreview else { return }
        providerLoading = true
        providerError = nil
        defer { if !Task.isCancelled && providerTask == task { providerLoading = false } }
        for attempt in 0..<24 {
            let result: Result<ProviderAnalyticsReport, Error> = await withCheckedContinuation { continuation in
                store.readProviderAnalytics(refresh: task.refreshID > 0 && attempt == 0) { continuation.resume(returning: $0) }
            }
            guard !Task.isCancelled, providerTask == task else { return }
            switch result {
            case .success(let report):
                sources = AnalyticsProviderObservation(identity: task.identity, report: report)
                if !report.sources.contains(where: \.refreshing) { return }
            case .failure(let error):
                providerError = error.localizedDescription
                return
            }
            if attempt < 23 { try? await Task.sleep(for: .seconds(2)) }
        }
        providerError = "部分提供商数据仍在后台查询，可稍后刷新。"
    }
    private func refresh() { range = period.timeRange(); load() }
    private func load() {
        store.loadAnalytics(AnalyticsQuery(fromMs: range.boundaries.first!, toMs: range.boundaries.last!, bucketBoundariesMs: range.boundaries, providerId: scope?.providerID, modelRecordKey: scope?.modelRecordKey, requestLimit: requestLimit))
    }
    private func showRequests(_ row: AnalyticsBreakdown) {
        scope = AnalyticsScope(name: row.name, providerID: row.providerId, modelRecordKey: row.modelRecordKey)
        requestLimit = 100; page = .requests; load()
    }
    private func showRequestDetail(_ id: String?) {
        if let id, let value = store.analyticsReport?.requests.first(where: { $0.requestId == id }) { requestDetail = AnalyticsRequestRow(value: value) }
    }
    private func reportContext(_ report: AnalyticsReport) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("\(loadedRange?.caption ?? "")，\(loadedScopeName)").lineLimit(1).truncationMode(.middle).help(loadedRange?.description ?? "")
                Spacer()
                if let updated = store.analyticsUpdatedAt { Text("更新于 \(updated.formatted(date: .omitted, time: .shortened))") }
            }.uiFont(.caption).foregroundStyle(.secondary)
            if store.analyticsIsLoading || report.storageWarning != nil {
                HStack(alignment: .firstTextBaseline) {
                    if store.analyticsIsLoading { Text("正在读取新范围，当前仍显示上次结果。").foregroundStyle(.secondary) }
                    else { Label("部分用量记录未能保存。", systemImage: "exclamationmark.triangle") }
                    Spacer()
                    if report.storageWarning != nil { ProblemsButton(store: store) }
                }.uiFont(.callout)
            }
        }.padding(.horizontal, 20).padding(.bottom, 12)
    }
    private func overview(_ report: AnalyticsReport?) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                VStack(alignment: .leading, spacing: 12) {
                    Text("用量").uiFont(.headline)
                    ViewThatFits(in: .horizontal) {
                        HStack(alignment: .top, spacing: 28) {
                            gatewayUsage(report)
                            providerUsage
                        }.fixedSize(horizontal: true, vertical: false)
                        VStack(alignment: .leading, spacing: 16) {
                            gatewayUsage(report)
                            providerUsage
                        }
                    }
                }
                if let report, report.overview.requestCount > 0 {
                    Divider()
                    HStack(alignment: .top, spacing: 28) {
                        VStack(alignment: .leading, spacing: 6) {
                            Text("\(report.overview.requestCount.formatted()) 次网关请求").uiFont(.headline)
                            Text(requestSummary(report.overview)).uiFont(.callout).foregroundStyle(.secondary)
                        }.frame(maxWidth: .infinity, alignment: .leading)
                        metric("网关有效输出速率", value: AnalyticsFormatting.speed(report.overview.outputTokensPerSecond), coverage: "\(report.overview.eligibleSpeedCount) / \(report.overview.requestCount) 次请求可计算")
                            .help(AnalyticsFormatting.speedDefinition)
                    }
                    if (loadedRange?.boundaries.count ?? 0) > 2 { trend(report) }
                }
                ImmediateDisclosureGroup {
                    if let totals = report?.overview, totals.requestCount > 0 {
                        HStack {
                            Text("网关完整报告总量")
                            Text(AnalyticsFormatting.tokens(totals.totalTokens) + " Token").monospacedDigit()
                            Text(coverage(totals.totalReportedCount, totals.requestCount)).foregroundStyle(.secondary)
                        }.uiFont(.callout)
                    }
                    Text("提供商用量按账户或组织报告展示，可能已包含网关用量。请求详情和性能来自网关。")
                    Text(AnalyticsFormatting.totalDefinition)
                    Text(AnalyticsFormatting.speedDefinition)
                    Text("— 表示未知，0 表示已报告为零。")
                } label: { Text("指标与数据来源说明") }
                .uiFont(.caption).foregroundStyle(.secondary)
            }.padding(20)
        }
    }

    private func gatewayUsage(_ report: AnalyticsReport?) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("网关记录").uiFont(.subheadline.weight(.semibold))
            if let totals = report?.overview, totals.requestCount > 0 {
                HStack(alignment: .top, spacing: 20) {
                    metric("输入 Token", value: AnalyticsFormatting.tokens(totals.inputTokens), coverage: coverage(totals.inputReportedCount, totals.requestCount))
                    metric("输出 Token", value: AnalyticsFormatting.tokens(totals.outputTokens), coverage: coverage(totals.outputReportedCount, totals.requestCount))
                }
            } else {
                Text(store.analyticsReadError == nil ? "所选范围暂无网关请求" : "暂无可显示的网关记录")
                    .uiFont(.callout).foregroundStyle(.secondary)
            }
        }.frame(minWidth: 224, alignment: .leading)
    }

    private var providerUsage: some View {
        let displayRange = loadedRange ?? range
        return AnalyticsProviderUsageSummary(report: providerReport, from: displayRange.start, to: displayRange.end,
            reading: providerLoading, problem: providerError,
            showSources: { showsProviderSources = true },
            showModels: { breakdownMode = .model; page = .breakdown }, retry: { providerReadID += 1 })
        .popover(isPresented: $showsProviderSources, arrowEdge: .bottom) {
            ProviderAnalyticsContent(report: providerReport, reading: providerLoading, problem: providerError) {
                providerReadID += 1
            }.frame(width: 560, height: 510)
        }
    }

    private func metric(_ title: String, value: String, coverage: String) -> some View {
        VStack(alignment: .leading, spacing: 6) { Text(title).uiFont(.callout).foregroundStyle(.secondary); Text(value).uiFont(.title2).monospacedDigit().lineLimit(1).minimumScaleFactor(0.8).help(value); Text(coverage).uiFont(.caption).foregroundStyle(.secondary) }.frame(maxWidth: .infinity, alignment: .leading)
    }
    private func requestSummary(_ totals: AnalyticsTotals) -> String {
        var parts = ["\(totals.completedCount) 次完成"]
        if totals.failedCount > 0 { parts.append("\(totals.failedCount) 次失败") }
        if totals.cancelledCount > 0 { parts.append("\(totals.cancelledCount) 次取消") }
        if totals.incompleteCount > 0 { parts.append("\(totals.incompleteCount) 次未完成") }
        if totals.rejectedCount > 0 { parts.append("\(totals.rejectedCount) 次拒绝") }
        return parts.joined(separator: "，")
    }
    private func coverage(_ known: UInt64, _ total: UInt64) -> String { "\(known) / \(total) 次请求报告" }
    private func trend(_ report: AnalyticsReport) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text("网关每日趋势").uiFont(.headline)
                Spacer()
                Picker("趋势指标", selection: $chartMetric) { ForEach(AnalyticsChartMetric.allCases) { Text($0.title).tag($0) } }.pickerStyle(.segmented).labelsHidden().frame(width: 190).accessibilityLabel("趋势指标")
            }
            if hasChartData(report) {
                Chart(report.trend.map(AnalyticsBucketRow.init)) { row in
                    let date = Date(timeIntervalSince1970: Double(row.value.fromMs) / 1_000)
                    if chartMetric == .tokens {
                        if let tokens = row.value.totals.inputTokens { BarMark(x: .value("日期", date, unit: .day), y: .value("Token", Double(tokens))).foregroundStyle(by: .value("用量", "输入")).position(by: .value("用量", "输入")) }
                        if let tokens = row.value.totals.outputTokens { BarMark(x: .value("日期", date, unit: .day), y: .value("Token", Double(tokens))).foregroundStyle(by: .value("用量", "输出")).position(by: .value("用量", "输出")) }
                    } else if let speed = row.value.totals.outputTokensPerSecond { PointMark(x: .value("日期", date), y: .value("Token/s", speed)).foregroundStyle(.tint) }
                }
                .chartXScale(domain: (loadedRange?.start ?? range.start)...(loadedRange ?? range).plotEnd)
                .chartXSelection(value: $chartSelection)
                .chartYAxis { AxisMarks { value in AxisGridLine(); AxisTick(); if let number = value.as(Double.self) { AxisValueLabel(TokenCount.rate(number)) } } }
                .chartXAxis { AxisMarks { value in AxisGridLine(); AxisTick(); if let date = value.as(Date.self) { AxisValueLabel(date.formatted(.dateTime.month(.twoDigits).day(.twoDigits))) } } }
                .frame(height: 175)
            } else { ContentUnavailableView(chartMetric == .tokens ? "暂无已报告的 Token 用量" : "暂无可计算的有效输出速率", systemImage: "chart.bar.xaxis").frame(height: 175) }
            if let chartSelection, let bucket = report.trend.first(where: { Double($0.fromMs) / 1_000 <= chartSelection.timeIntervalSince1970 && Double($0.toMs) / 1_000 > chartSelection.timeIntervalSince1970 }) {
                Text("\((loadedRange ?? range).dayLabel(Date(timeIntervalSince1970: Double(bucket.fromMs) / 1_000)))，输入：\(AnalyticsFormatting.tokens(bucket.totals.inputTokens))，输出：\(AnalyticsFormatting.tokens(bucket.totals.outputTokens))，有效输出速率：\(AnalyticsFormatting.speed(bucket.totals.outputTokensPerSecond))").uiFont(.caption).foregroundStyle(.secondary)
            } else { Text("输入、输出按各自报告绘制；未报告不补零。速率包含等待时间。").uiFont(.caption).foregroundStyle(.secondary) }
        }
    }
    private func hasChartData(_ report: AnalyticsReport) -> Bool {
        report.trend.contains { chartMetric == .tokens ? $0.totals.inputTokens != nil || $0.totals.outputTokens != nil : $0.totals.outputTokensPerSecond != nil }
    }
    private func breakdown(_ report: AnalyticsReport?) -> some View {
        VStack(spacing: 0) {
            HStack {
                Picker("分解方式", selection: $breakdownMode) {
                    ForEach(AnalyticsBreakdownMode.allCases) { Text($0.title).tag($0) }
                }.pickerStyle(.segmented).labelsHidden().frame(width: 190)
                Spacer()
            }.padding(.horizontal, 20).padding(.bottom, 12)
            let displayRange = loadedRange ?? range
            AnalyticsUsageBreakdown(local: report, provider: providerReport,
                fromMs: displayRange.boundaries.first!, toMs: displayRange.boundaries.last!,
                byModel: breakdownMode == .model, showRequests: showRequests)
        }
    }
    private func requestList(_ report: AnalyticsReport) -> some View {
        VStack(spacing: 0) {
            HStack {
                Text("筛选：\(loadedScopeName)").uiFont(.callout)
                if scope != nil || store.analyticsLoadedQuery?.providerId != nil || store.analyticsLoadedQuery?.modelRecordKey != nil { Button("清除筛选") { scope = nil; requestLimit = 100; load() } }
                Spacer()
                Button("显示详情…") { showRequestDetail(selectedRequestID) }.disabled(selectedRequestID == nil)
            }.padding(.horizontal, 20).padding(.bottom, 12)
            Table(report.requests.map(AnalyticsRequestRow.init), selection: $selectedRequestID) {
                TableColumn("时间") { row in Text(Date(timeIntervalSince1970: Double(row.value.startedAtMs) / 1_000), format: .dateTime.month(.twoDigits).day(.twoDigits).hour().minute()).monospacedDigit() }.width(125)
                TableColumn("提供商 / 模型") { row in VStack(alignment: .leading, spacing: 3) { Text(row.value.providerName); Text(row.value.providerModelId).uiFont(.caption).foregroundStyle(.secondary) }.lineLimit(1) }
                TableColumn("结果") { row in Text(AnalyticsFormatting.outcome(row.value.outcome)) }.width(70)
                TableColumn("输出 Token") { row in Text(AnalyticsFormatting.tokens(row.value.outputTokens)).monospacedDigit() }.width(100)
            }.contextMenu(forSelectionType: String.self) { ids in if let id = ids.first { Button("显示详情…") { showRequestDetail(id) } } } primaryAction: { ids in showRequestDetail(ids.first) }
            // Reusing this coordinator for filtered-to-all replacement produced an
            // NSTableView delegate reentrancy warning. Key only by accepted scope;
            // same-scope refresh still retains its table and request-ID selection.
            .id(requestTableScope)
            HStack {
                Text("最近 \(report.requests.count) / \(report.overview.requestCount) 条请求记录").uiFont(.caption).foregroundStyle(.secondary)
                Spacer()
                if report.overview.requestCount > UInt64(report.requests.count) && requestLimit < 500 { Button("加载更多") { requestLimit = min(500, requestLimit + 100); load() } }
                else if report.overview.requestCount > UInt64(report.requests.count) { Text("最多展示最近 500 条").uiFont(.caption).foregroundStyle(.secondary) }
            }.padding(12)
        }
    }
}

private struct AnalyticsRequestDetail: View {
    let request: AnalyticsRequest
    @Environment(\.dismiss) private var dismiss
    var body: some View {
        VStack(spacing: 0) {
            HStack { Text("请求详情").uiFont(.headline); Spacer(); Button("完成") { dismiss() }.keyboardShortcut(.cancelAction) }.padding(20)
            Divider()
            ScrollView {
                VStack(alignment: .leading, spacing: 20) {
                    Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 10) {
                        row("结果", AnalyticsFormatting.outcome(request.outcome))
                        row("提供商", request.providerName)
                        row("模型 ID", request.providerModelId)
                        row("协议", AnalyticsFormatting.protocolName(request.protocol))
                        row("开始", Date(timeIntervalSince1970: Double(request.startedAtMs) / 1_000).formatted())
                        row("输入 Token", AnalyticsFormatting.tokens(request.inputTokens))
                        row("输出 Token", AnalyticsFormatting.tokens(request.outputTokens))
                        row("有效输出速率", AnalyticsFormatting.speed(request.outputTokensPerSecond))
                        row("请求耗时", AnalyticsFormatting.duration(request.elapsedMs))
                        row("首输出延迟", AnalyticsFormatting.duration(request.firstOutputMs))
                    }
                    ImmediateDisclosureGroup {
                        Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 10) {
                            row("推理 Token", AnalyticsFormatting.tokens(request.reasoningOutputTokens))
                            row("普通输入", AnalyticsFormatting.tokens(request.uncachedInputTokens))
                            row("缓存输入", AnalyticsFormatting.tokens(request.cachedInputTokens))
                            row("缓存读取", AnalyticsFormatting.tokens(request.cacheReadInputTokens))
                            row("缓存创建", AnalyticsFormatting.tokens(request.cacheCreationInputTokens))
                        }
                        Text("缓存字段按上游协议保留，不再次加到已归一的输入中；推理 Token 属于输出子集。").uiFont(.caption).foregroundStyle(.secondary)
                    } label: { Text("推理与缓存用量") }
                    ImmediateDisclosureGroup {
                        Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 10) {
                            row("请求编号", request.requestId)
                            row("HTTP 状态", request.status.map { String($0) } ?? "—")
                            row("业务终态耗时", AnalyticsFormatting.duration(request.terminalElapsedMs))
                            row("用量报告", request.usageReported ? request.usageComplete ? "完整" : "部分" : "未报告")
                        }
                    } label: { Text("请求信息") }
                    ImmediateDisclosureGroup {
                        Text(AnalyticsFormatting.speedDefinition)
                        Text("首输出延迟从上游派发到首次正文、推理或工具内容；非流式响应未知。请求耗时涵盖完成接收、失败或取消；速率仅使用可确认的业务终态耗时。— 表示未知。")
                    } label: { Text("指标说明") }
                    .uiFont(.caption).foregroundStyle(.secondary)
                }.padding(20).textSelection(.enabled)
            }
        }.frame(width: 500, height: 540)
    }
    private func row(_ title: String, _ value: String) -> some View { GridRow { Text(title).foregroundStyle(.secondary); Text(value).frame(maxWidth: .infinity, alignment: .leading) } }
}

private enum AnalyticsPage: String, CaseIterable, Identifiable { case overview, breakdown, requests; var id: Self { self }; var title: String { switch self { case .overview: return "概览"; case .breakdown: return "提供商与模型"; case .requests: return "请求" } } }
private enum AnalyticsFormatting {
    static let totalDefinition = "总量单位为 Token，仅合计输入和输出均已报告的请求。输入、输出列分别合计各自已报告部分，覆盖可能不同，不能直接相加比较。"
    static let speedDefinition = "有效输出速率 = 已报告输出 Token ÷ 上游派发至业务终态的请求耗时，包含等待；汇总按可计算请求加权，不是单次速率的平均值。"
    static func tokens(_ value: UInt64?) -> String { value.map { TokenCount.format($0) } ?? "—" }
    static func speed(_ value: Double?) -> String { guard let value, value.isFinite else { return "—" }; return TokenCount.rate(value) + " Token/s" }
    static func duration(_ value: UInt64?) -> String { value.map { Duration.milliseconds(Double($0)).formatted(.units(allowed: [.minutes, .seconds], width: .abbreviated, fractionalPart: .show(length: 2))) } ?? "—" }
    static func outcome(_ value: AnalyticsOutcome) -> String { switch value { case .completed: return "完成"; case .incomplete: return "未完成"; case .failed: return "失败"; case .cancelled: return "取消"; case .rejected: return "拒绝" } }
    static func protocolName(_ value: ApiProtocol) -> String { switch value { case .chatCompletions: return "OpenAI ChatCompletions"; case .responses: return "OpenAI Responses"; case .messages: return "Anthropic Messages"; case .gemini: return "Gemini GenerateContent" } }
}
private struct AnalyticsScope { var name: String; var providerID: String?; var modelRecordKey: String? }
private struct AnalyticsProviderObservation {
    let identity: ProviderReportReadIdentity
    let report: ProviderAnalyticsReport
}
private struct AnalyticsProviderTask: Equatable {
    let identity: ProviderReportReadIdentity
    let refreshID: Int
}
private struct AnalyticsBucketRow: Identifiable { let value: AnalyticsBucket; var id: Int64 { value.fromMs } }
private struct AnalyticsRequestTableScope: Hashable { let providerID: String?; let modelRecordKey: String? }
private struct AnalyticsRequestRow: Identifiable { let value: AnalyticsRequest; var id: String { value.requestId } }
private enum AnalyticsBreakdownMode: String, CaseIterable, Identifiable { case provider, model; var id: Self { self }; var title: String { self == .provider ? "提供商" : "模型" } }
private enum AnalyticsChartMetric: String, CaseIterable, Identifiable { case tokens, speed; var id: Self { self }; var title: String { self == .tokens ? "Token 用量" : "有效输出速率" } }
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
    // A partial current-day query still needs the whole native day bucket's drawing space.
    // Query bounds and displayed coverage remain unchanged; no future records are created.
    var plotEnd: Date {
        var calendar = Calendar.current
        calendar.timeZone = timeZone
        let midnight = calendar.startOfDay(for: end)
        return end > midnight ? calendar.date(byAdding: .day, value: 1, to: midnight)! : end
    }
    var caption: String { "\(dayLabel(start))–\(dayLabel(end))（\(timeZone.identifier)）" }
    var description: String { "\(instantLabel(start)) 至 \(instantLabel(end))（不含结束时刻）" }
    private func instantLabel(_ date: Date) -> String { var style = Date.FormatStyle(date: .abbreviated, time: .shortened); style.timeZone = timeZone; return date.formatted(style) }
    func dayLabel(_ date: Date) -> String { var style = Date.FormatStyle(date: .abbreviated, time: .omitted); style.timeZone = timeZone; return date.formatted(style) }
}
