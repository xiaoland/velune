import SwiftUI
import VeluneBindings

/// Where the loaded conversation stands, from its turns' native records:
/// context is the latest turn's, counts are summed over the turns that
/// recorded them, and limits and cost are the latest reported.
struct ConversationStatus: Equatable {
    var conversationID: String?
    var endedTurns: Int
    var contextTokens: UInt64?
    var contextWindow: UInt64?
    var inputTokens: UInt64?
    var cacheReadTokens: UInt64?
    var outputTokens: UInt64?
    var reasoningTokens: UInt64?
    var unrecordedTurns: Int
    var directory: String?
    var costMicroUSD: UInt64?

    var contextFraction: Double? {
        guard let tokens = contextTokens, let window = contextWindow, window > 0 else { return nil }
        return min(Double(tokens) / Double(window), 1)
    }
    var sessionTokens: UInt64? {
        guard inputTokens != nil || outputTokens != nil else { return nil }
        return (inputTokens ?? 0) + (outputTokens ?? 0)
    }
    func has(_ item: BindingStatusLineItem) -> Bool {
        switch item {
        case .contextUsage: contextTokens != nil
        case .sessionTokens: sessionTokens != nil
        case .workingDirectory: directory != nil
        case .rateLimits: false
        case .sessionCost: costMicroUSD != nil
        }
    }
}

/// The composer's status line: the pinned items in a compact row; hovering
/// explains them and a click lists everything the records say.
struct ComposerStatusLine: View {
    @ObservedObject var store: AppStore
    @State private var branch: String?
    @State private var showsDetails = false

    private var status: ConversationStatus { store.conversationStatus }
    private var accounts: [ComposerAccountQuota] { store.composerAccountQuotas }

    var body: some View {
        let status = self.status
        let items = store.statusLine.filter { $0 == .rateLimits ? !accounts.isEmpty || store.composerQuotaError != nil : status.has($0) }
        HStack(spacing: 0) {
            if !items.isEmpty || !accounts.isEmpty || store.composerQuotaError != nil {
                Button { showsDetails.toggle() } label: {
                    HStack(spacing: 10) {
                        if items.isEmpty { Text("状态") }
                        ForEach(items, id: \.self) { compact($0, status) }
                    }
                        .font(.callout).monospacedDigit().foregroundStyle(.secondary).lineLimit(1)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.borderless)
                .help(summary(items, status))
                .accessibilityLabel("会话状态")
                .accessibilityValue(summary(items, status))
                .popover(isPresented: $showsDetails, arrowEdge: .top) {
                    StatusDetails(status: status, accounts: accounts, branch: branch, display: store.contextUsageDisplay, quotaDisplay: store.usageDisplay, quotaProblem: store.composerQuotaError)
                }
            }
        }
        .task(id: store.composerQuotaRequest) {
            await store.observeComposerQuota(store.composerQuotaRequest)
        }
        // The branch can change with any turn; reading it is one git call.
        .task(id: "\(status.conversationID ?? "")|\(status.directory ?? "")|\(status.endedTurns)") {
            guard status.directory != nil else { branch = nil; return }
            store.readWorkingBranch { branch = $0 }
        }
    }

    @ViewBuilder private func compact(_ item: BindingStatusLineItem, _ status: ConversationStatus) -> some View {
        switch item {
        case .contextUsage:
            if let fraction = status.contextFraction {
                HStack(spacing: 4) { ContextRing(fraction: fraction, display: store.contextUsageDisplay); Text(StatusFormat.context(fraction, store.contextUsageDisplay)) }
            } else if let tokens = status.contextTokens {
                Text("上下文 \(StatusFormat.short(tokens))")
            }
        case .sessionTokens:
            if let tokens = status.sessionTokens { Label(StatusFormat.short(tokens), systemImage: "sum").labelStyle(.titleAndIcon) }
        case .workingDirectory:
            if let directory = status.directory {
                HStack(spacing: 4) {
                    Label(URL(fileURLWithPath: directory).lastPathComponent, systemImage: "folder")
                    if let branch { Label(branch, systemImage: "arrow.triangle.branch") }
                }.truncationMode(.middle)
            }
        case .rateLimits:
            let text = store.composerQuotaError.map { _ in "额度读取失败" } ?? accountSummary(short: true)
            Label(text, systemImage: "gauge.with.dots.needle.33percent")
        case .sessionCost:
            if let cost = status.costMicroUSD { Text(StatusFormat.cost(cost)) }
        }
    }

    private func summary(_ items: [BindingStatusLineItem], _ status: ConversationStatus) -> String {
        let lines = items.compactMap { item -> String? in
            switch item {
            case .contextUsage:
                guard let tokens = status.contextTokens else { return nil }
                guard let window = status.contextWindow, let fraction = status.contextFraction else { return "上下文：已用 \(TokenCount.format(tokens)) tokens，模型窗口未知" }
                if store.contextUsageDisplay == .remaining { return "上下文：剩余 \(TokenCount.format(window.subtractingClamped(tokens))) / \(TokenCount.format(window)) tokens（\(StatusFormat.percent(1 - fraction))）" }
                return "上下文：已用 \(TokenCount.format(tokens)) / \(TokenCount.format(window)) tokens（\(StatusFormat.percent(fraction))）"
            case .sessionTokens: return status.sessionTokens.map { "本会话累计：\(TokenCount.format($0)) tokens" }
            case .workingDirectory: return status.directory.map { "工作目录：\($0)" + (branch.map { "（\($0)）" } ?? "") }
            case .rateLimits:
                return store.composerQuotaError ?? accountSummary(short: false)
            case .sessionCost: return status.costMicroUSD.map { "费用估算：\(StatusFormat.cost($0))" }
            }
        }
        return (lines + ["点按查看会话状态"]).joined(separator: "\n")
    }

    private func accountSummary(short: Bool) -> String {
        accounts.map { account in
            let windows = account.windows.map { window in
                let used = min(max(window.usedPercent, 0), 100)
                let label = UsageFormat.label(Int((store.usageDisplay == .remaining ? 100 - used : used).rounded()), store.usageDisplay)
                return (account.isRecorded && window.period == nil ? "限额" : QuotaWindow.name(window.id)) + " " + label + (short ? "" : "，" + QuotaResetFormat.text(window.resetsAt))
            }
            let balances = (account.quota.balances).map { $0.title + " " + $0.formatted }
            let values = windows + balances
            let state = !account.isRecorded && account.quota.freshness == .stale ? "（额度已过期）" : ""
            return account.title + state + "：" + (values.isEmpty ? QuotaStatusText.summary(account.quota) : values.joined(separator: " · "))
        }.joined(separator: short ? " · " : "\n")
    }
}

/// How full the context is: a thin ring in the secondary colour, filling
/// with use or emptying as it is used up, that turns orange, then red, as it
/// nears the window.
private struct ContextRing: View {
    let fraction: Double
    let display: BindingUsageDisplay
    var body: some View {
        ZStack {
            Circle().stroke(.quaternary, lineWidth: 2)
            Circle().trim(from: 0, to: display == .remaining ? 1 - fraction : fraction)
                .stroke(fraction >= 0.9 ? Color.red : fraction >= 0.75 ? Color.orange : Color.secondary,
                        style: StrokeStyle(lineWidth: 2, lineCap: .round))
                .rotationEffect(.degrees(-90))
        }
        .frame(width: 12, height: 12)
        .accessibilityHidden(true)
    }
}

/// Recorded conversation usage and the selected accounts' quota, whether pinned or not.
struct StatusDetails: View {
    let status: ConversationStatus
    var accounts: [ComposerAccountQuota] = []
    let branch: String?
    let display: BindingUsageDisplay
    let quotaDisplay: BindingUsageDisplay
    var quotaProblem: String? = nil
    var body: some View {
        ViewThatFits(in: .vertical) {
            details
            ScrollView { details }
        }
        .frame(width: 380).frame(maxHeight: 600).fixedSize(horizontal: false, vertical: true)
    }
    private var details: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("会话状态").font(.headline)
            Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 6) {
                if let tokens = status.contextTokens {
                    heading("上下文")
                    if display == .remaining, let window = status.contextWindow {
                        row("剩余", "\(TokenCount.format(window.subtractingClamped(tokens))) / \(TokenCount.format(window)) tokens")
                    } else {
                        row("已用", status.contextWindow.map { "\(TokenCount.format(tokens)) / \(TokenCount.format($0)) tokens" } ?? "\(TokenCount.format(tokens)) tokens（模型窗口未知）")
                    }
                    if let fraction = status.contextFraction {
                        GridRow {
                            Color.clear.gridCellUnsizedAxes([.horizontal, .vertical])
                            HStack {
                                ProgressView(value: display == .remaining ? 1 - fraction : fraction).tint(fraction >= 0.9 ? .red : fraction >= 0.75 ? .orange : nil)
                                Text(StatusFormat.context(fraction, display)).foregroundStyle(.secondary).monospacedDigit()
                            }
                        }
                    }
                }
                if status.sessionTokens != nil {
                    heading("本会话累计")
                    if let value = status.inputTokens { row("输入", TokenCount.format(value)) }
                    if let value = status.cacheReadTokens { row("缓存读取", TokenCount.format(value)) }
                    if let value = status.outputTokens { row("输出", TokenCount.format(value)) }
                    if let value = status.reasoningTokens { row("推理", TokenCount.format(value)) }
                    if status.unrecordedTurns > 0 { row("未记录", "\(status.unrecordedTurns) 轮") }
                }
                if let directory = status.directory {
                    heading("工作目录")
                    row("路径", (directory as NSString).abbreviatingWithTildeInPath)
                    if let branch { row("分支", branch) }
                }
                if let cost = status.costMicroUSD {
                    heading("费用")
                    row("估算", StatusFormat.cost(cost))
                }
            }
            if let quotaProblem { Divider(); Text(quotaProblem).foregroundStyle(.secondary) }
            ForEach(accounts) { account in
                Divider()
                Text(account.title).font(.subheadline.weight(.semibold))
                if account.quota.available == false { Text("账户当前不可用").font(.caption).foregroundStyle(.orange) }
                ForEach(Array(account.windows.enumerated()), id: \.offset) { _, window in
                    QuotaWindowRow(window: window, limit: account.limit(for: window.id),
                                   fresh: account.quota.freshness == .fresh, display: quotaDisplay, showsModels: true, title: account.isRecorded && window.period == nil ? "限额" : nil)
                }
                ForEach(Array((account.quota.balances).enumerated()), id: \.offset) { _, balance in
                    QuotaBalanceRow(balance: balance, fresh: account.quota.freshness == .fresh)
                }
                if account.isRecorded {
                    Text("会话记录，非实时账户查询").font(.caption).foregroundStyle(.secondary)
                } else { QuotaStatusText(quota: account.quota) }
            }
            Text("会话用量取自运行时记录；累计为已记录各轮之和，输入含缓存，输出含推理；费用为运行时估算，不代表实际账单。")
                .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            SettingsTabLink(tab: .conversation, title: "自定义状态栏…")
        }
        .padding(14).frame(width: 380, alignment: .leading)
    }
    private func heading(_ title: String) -> some View {
        GridRow { Text(title).font(.subheadline.weight(.semibold)).gridCellColumns(2).padding(.top, 4) }
    }
    private func row(_ label: String, _ value: String) -> some View {
        GridRow {
            Text(label).foregroundStyle(.secondary).gridColumnAlignment(.trailing)
            Text(value).monospacedDigit().textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
        }
    }
}

enum StatusFormat {
    /// Whole percent; a context barely used still reads as used.
    static func percent(_ fraction: Double) -> String {
        fraction > 0 && fraction < 0.01 ? "<1%" : fraction.formatted(.percent.precision(.fractionLength(0)))
    }
    /// The context as the user reads it, from the used fraction; the used
    /// share stays bare, as it always read.
    static func context(_ used: Double, _ display: BindingUsageDisplay) -> String {
        guard display == .remaining else { return percent(used) }
        // Barely used still reads as not all remaining.
        let remaining = used > 0 && used < 0.01 ? ">99%" : percent(1 - used)
        return "剩余 \(remaining)"
    }
    /// A rate limit's window, used or remaining.
    static func limit(_ usedPercent: UInt32, _ display: BindingUsageDisplay, short: Bool) -> String {
        let used = min(usedPercent, 100)
        switch display {
        case .used: return short ? "\(used)%" : "已用 \(used)%"
        case .remaining: return "剩余 \(100 - used)%"
        }
    }
    static func limitDetail(_ value: RateLimit, _ display: BindingUsageDisplay) -> String {
        let reset = value.resetsAtUnixMs.map { Date(timeIntervalSince1970: Double($0) / 1000) }
        return limit(value.usedPercent, display, short: false) + "，" + QuotaResetFormat.text(reset)
    }
    static func short(_ tokens: UInt64) -> String { TokenCount.format(tokens) }
    static func cost(_ microUSD: UInt64) -> String {
        (Double(microUSD) / 1_000_000).formatted(.currency(code: "USD").precision(.fractionLength(microUSD < 10_000 ? 4 : 2)))
    }
    /// A limit's window by its length: hours or days when whole.
    static func window(_ minutes: UInt64?, short: Bool) -> String {
        guard let minutes, minutes > 0 else { return short ? "限额" : "限额窗口" }
        if minutes % 1440 == 0 { return short ? "\(minutes / 1440)d" : "\(minutes / 1440) 天窗口" }
        if minutes % 60 == 0 { return short ? "\(minutes / 60)h" : "\(minutes / 60) 小时窗口" }
        return short ? "\(minutes)m" : "\(minutes) 分钟窗口"
    }
}

extension UInt64 {
    /// What is left of this many after `used`, never below zero.
    func subtractingClamped(_ used: UInt64) -> UInt64 { used >= self ? 0 : self - used }
}
