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
    var rateLimits: [RateLimit]
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
        case .rateLimits: !rateLimits.isEmpty
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

    var body: some View {
        let status = store.conversationStatus
        let items = store.statusLine.filter(status.has)
        Group {
            if !items.isEmpty {
                Button { showsDetails.toggle() } label: {
                    HStack(spacing: 10) { ForEach(items, id: \.self) { compact($0, status) } }
                        .font(.callout).monospacedDigit().foregroundStyle(.secondary).lineLimit(1)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.borderless)
                .help(summary(items, status))
                .accessibilityLabel("会话状态")
                .accessibilityValue(summary(items, status))
                .popover(isPresented: $showsDetails, arrowEdge: .top) { StatusDetails(status: status, branch: branch) }
            }
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
                HStack(spacing: 4) { ContextRing(fraction: fraction); Text(StatusFormat.percent(fraction)) }
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
            Label(status.rateLimits.map { "\(StatusFormat.window($0.windowMinutes, short: true)) \($0.usedPercent)%" }.joined(separator: " · "),
                  systemImage: "gauge.with.dots.needle.33percent")
        case .sessionCost:
            if let cost = status.costMicroUSD { Text(StatusFormat.cost(cost)) }
        }
    }

    private func summary(_ items: [BindingStatusLineItem], _ status: ConversationStatus) -> String {
        let lines = items.compactMap { item -> String? in
            switch item {
            case .contextUsage:
                guard let tokens = status.contextTokens else { return nil }
                guard let window = status.contextWindow, let fraction = status.contextFraction else { return "上下文：已用 \(tokens.formatted()) tokens，模型窗口未知" }
                return "上下文：已用 \(tokens.formatted()) / \(window.formatted()) tokens（\(StatusFormat.percent(fraction))）"
            case .sessionTokens: return status.sessionTokens.map { "本会话累计：\($0.formatted()) tokens" }
            case .workingDirectory: return status.directory.map { "工作目录：\($0)" + (branch.map { "（\($0)）" } ?? "") }
            case .rateLimits: return "账户限额：" + status.rateLimits.map { "\(StatusFormat.window($0.windowMinutes, short: false))已用 \($0.usedPercent)%" }.joined(separator: "，")
            case .sessionCost: return status.costMicroUSD.map { "费用估算：\(StatusFormat.cost($0))" }
            }
        }
        return (lines + ["点按查看会话状态"]).joined(separator: "\n")
    }
}

/// How full the context is: a thin ring in the secondary colour that turns
/// orange, then red, as it nears the window.
private struct ContextRing: View {
    let fraction: Double
    var body: some View {
        ZStack {
            Circle().stroke(.quaternary, lineWidth: 2)
            Circle().trim(from: 0, to: fraction)
                .stroke(fraction >= 0.9 ? Color.red : fraction >= 0.75 ? Color.orange : Color.secondary,
                        style: StrokeStyle(lineWidth: 2, lineCap: .round))
                .rotationEffect(.degrees(-90))
        }
        .frame(width: 12, height: 12)
        .accessibilityHidden(true)
    }
}

/// Everything the records say about the conversation, whether pinned or not.
private struct StatusDetails: View {
    let status: ConversationStatus
    let branch: String?
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("会话状态").font(.headline)
            Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 6) {
                if let tokens = status.contextTokens {
                    heading("上下文")
                    row("已用", status.contextWindow.map { "\(tokens.formatted()) / \($0.formatted()) tokens" } ?? "\(tokens.formatted()) tokens（模型窗口未知）")
                    if let fraction = status.contextFraction {
                        GridRow {
                            Color.clear.gridCellUnsizedAxes([.horizontal, .vertical])
                            HStack {
                                ProgressView(value: fraction).tint(fraction >= 0.9 ? .red : fraction >= 0.75 ? .orange : nil)
                                Text(StatusFormat.percent(fraction)).foregroundStyle(.secondary).monospacedDigit()
                            }
                        }
                    }
                }
                if status.sessionTokens != nil {
                    heading("本会话累计")
                    if let value = status.inputTokens { row("输入", value.formatted()) }
                    if let value = status.cacheReadTokens { row("缓存读取", value.formatted()) }
                    if let value = status.outputTokens { row("输出", value.formatted()) }
                    if let value = status.reasoningTokens { row("推理", value.formatted()) }
                    if status.unrecordedTurns > 0 { row("未记录", "\(status.unrecordedTurns) 轮") }
                }
                if let directory = status.directory {
                    heading("工作目录")
                    row("路径", (directory as NSString).abbreviatingWithTildeInPath)
                    if let branch { row("分支", branch) }
                }
                if !status.rateLimits.isEmpty {
                    heading("账户限额")
                    ForEach(Array(status.rateLimits.enumerated()), id: \.offset) { _, limit in
                        row(StatusFormat.window(limit.windowMinutes, short: false),
                            "已用 \(limit.usedPercent)%" + (limit.resetsAtUnixMs.map { "，\(Date(timeIntervalSince1970: Double($0) / 1000).formatted(.relative(presentation: .named)))重置" } ?? ""))
                    }
                }
                if let cost = status.costMicroUSD {
                    heading("费用")
                    row("估算", StatusFormat.cost(cost))
                }
            }
            Text("取自运行时的原生记录；累计为已记录各轮之和，输入含缓存，输出含推理；费用为运行时按其价目的估算，不代表实际账单。")
                .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            SettingsTabLink(tab: .conversation, title: "自定义状态栏…")
        }
        .padding(14).frame(width: 320, alignment: .leading)
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
    static func short(_ tokens: UInt64) -> String { tokens.formatted(.number.notation(.compactName)) }
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
