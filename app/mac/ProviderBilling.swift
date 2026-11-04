import SwiftUI
import VeluneBindings

/// Official organization reporting has its own credentials and never controls routing.
struct ProviderBillingView: View {
    @ObservedObject var store: DeviceStore
    let providerID: String
    let service: BillingService
    @Environment(\.dismiss) private var dismiss
    @State private var status: ProviderBillingStatus?
    @State private var enabled = false
    @State private var organization = ""
    @State private var scope = 0
    @State private var scopeID = ""
    @State private var key = ""
    @State private var confirmed = false
    @State private var loaded = false
    @State private var reading = false
    @State private var saving = false
    @State private var active = false
    @State private var problem: String?
    private var scopeName: String { service == .openAi ? "Project" : "Workspace" }
    private var valid: Bool {
        !enabled || (!organization.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && (scope == 0 || !scopeID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            && (!key.isEmpty || status?.authentication.configured == true) && confirmed)
    }
    var body: some View {
        VStack(spacing: 0) {
            HStack { Text("用量与账单").uiFont(.headline); Spacer(); Button("完成") { dismiss() }.keyboardShortcut(.cancelAction) }.padding(20)
            Divider()
            Form {
                Section {
                    Toggle("查询组织用量与账单", isOn: $enabled)
                    if enabled {
                        TextField("组织 ID", text: $organization)
                        Picker("统计范围", selection: $scope) {
                            Text("整个组织").tag(0)
                            Text(scopeName).tag(1)
                            Text("API key").tag(2)
                        }
                        if scope != 0 { TextField(scope == 1 ? "\(scopeName) ID" : "API key ID（非密钥）", text: $scopeID) }
                        SecureField(status?.authentication.configured == true ? "新的查询 Key（留空保留）" : "组织查询 Key", text: $key)
                        Toggle("我确认查询凭据属于此组织，且模型 Key 对应所选范围", isOn: $confirmed)
                        Text(service == .openAi ? "使用有查询权限的 OpenAI Admin Key，与模型 Key 分开保存。" : "使用有组织查询权限的 Admin Key。普通 workspace 模型 Key 不适用；个人账户不支持 Admin API。")
                            .uiFont(.caption).foregroundStyle(.secondary)
                        if status?.authentication.configured == true && status?.authentication.bindingConfirmed == false {
                            Text("模型认证已变化，请重新确认关联后保存。").foregroundStyle(.orange)
                        }
                    }
                    Button(saving ? "正在保存…" : "保存查询配置") { save() }.disabled(!loaded || saving || reading || !valid)
                    if let problem { Text(problem).foregroundStyle(.red).textSelection(.enabled) }
                } footer: {
                    Text("查询配置单独保存。关闭后删除查询凭据；报表不参与网关准入。组织总额包含此范围内所有客户端的消费，多个提供商引用同一范围时不要重复相加。")
                        .formFooterStyle()
                }
                if let status, status.configuration != nil {
                    Section {
                        HStack {
                            Text(status.refreshing ? "正在查询…" : status.stale ? "上次报表已过期" : "本月报表（UTC）")
                            Spacer()
                            Button("刷新") { read(refresh: true) }.disabled(reading || saving || status.refreshing)
                        }
                        if let issue = status.problem { Text(issue).foregroundStyle(.orange) }
                        if let report = status.report { reportContent(report) }
                        else if !status.refreshing { Text("尚未取得报表").foregroundStyle(.secondary) }
                    } footer: {
                        Text(service == .anthropic ? "官方 Analytics 当前为 beta，通常延迟 4 小时，最长 24 小时；费用为折扣后、抵扣 credits 前金额。空记录也可能由范围 ID 不匹配导致。" : "费用来自组织 Costs；文本生成用量来自 Completions Usage，二者覆盖的服务可能不同。报表可能延迟，不代表充值余额。")
                            .formFooterStyle()
                    }
                }
            }.formStyle(.grouped)
            Divider()
            Text("充值余额请在官方控制台查看；支出限额与速率上限分别展示，不推算剩余额度。")
                .uiFont(.caption).foregroundStyle(.secondary).frame(maxWidth: .infinity, alignment: .leading).padding(16)
        }.frame(width: 620, height: 640)
        .task {
            active = true
            read(refresh: false)
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(status?.refreshing == true ? 2 : 30))
                guard !Task.isCancelled else { break }
                read(refresh: false)
            }
        }
        .onDisappear { active = false; key = "" }
    }
    @ViewBuilder private func reportContent(_ report: BillingReport) -> some View {
        LabeledContent("已保存范围", value: scopeLabel(report.configuration))
        LabeledContent("统计区间", value: "\(date(report.startingAtMs)) 至 \(date(report.endingAtMs))")
        LabeledContent("本地抓取", value: date(report.observedAtMs))
        LabeledContent("官方数据更新", value: report.dataRefreshedAtMs.map(date) ?? "未报告")
        if let costs = report.costs {
            if costs.isEmpty { Text("费用：此范围暂无记录").foregroundStyle(.secondary) }
            ForEach(Array(costs.enumerated()), id: \.offset) { _, cost in LabeledContent("本月费用", value: money(cost.amount, cost.currency)) }
        }
        if let usage = report.usage {
            if usage.requests == 0 && usage.inputTokens == 0 && usage.outputTokens == 0 { Text("用量：此范围暂无记录").foregroundStyle(.secondary) }
            else {
                LabeledContent(service == .openAi ? "文本生成请求" : "请求数", value: usage.requests.formatted())
                LabeledContent("输入 Token（含缓存）", value: usage.inputTokens.formatted())
                LabeledContent("输出 Token", value: usage.outputTokens.formatted())
                if let cached = usage.cachedInputTokens { LabeledContent("其中缓存读取", value: cached.formatted()) }
                if let created = usage.cacheCreationInputTokens { LabeledContent("其中缓存创建", value: created.formatted()) }
            }
        }
        if let limit = report.spendLimit {
            LabeledContent("官方月度支出限额", value: money(limit.amount, limit.currency))
            Text(limit.enforced ? "官方已启用此限制；不表示当前已耗尽。" : "官方未启用此限制。")
                .uiFont(.caption).foregroundStyle(.secondary)
        }
        if let limits = report.rateLimits {
            ImmediateDisclosureGroup("配置速率上限（\(limits.count)）") {
                VStack(alignment: .leading, spacing: 8) {
                    ForEach(Array(limits.enumerated()), id: \.offset) { _, limit in
                        VStack(alignment: .leading, spacing: 2) {
                            Text("\(limit.name) · \(limit.limitType)：\(limit.value.formatted())")
                            if !limit.modelIds.isEmpty { Text(limit.modelIds.joined(separator: ", ")).uiFont(.caption).foregroundStyle(.secondary) }
                        }.textSelection(.enabled)
                    }
                }.padding(.top, 8)
            }
        }
        ForEach(report.problems, id: \.self) { Text($0).foregroundStyle(.orange) }
    }
    private func scopeLabel(_ configuration: BillingConfiguration) -> String {
        let suffix: String
        switch configuration.scope {
        case .organization: suffix = "整个组织"
        case .project(let id): suffix = "Project \(id)"
        case .workspace(let id): suffix = "Workspace \(id)"
        case .apiKey(let id): suffix = "API key \(id)"
        }
        return "\(configuration.organizationId) · \(suffix)"
    }
    private func date(_ milliseconds: Int64) -> String {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.timeZone = TimeZone(secondsFromGMT: 0)
        formatter.dateFormat = "yyyy-MM-dd HH:mm 'UTC'"
        return formatter.string(from: Date(timeIntervalSince1970: Double(milliseconds) / 1000))
    }
    private func money(_ amount: Double, _ currency: String) -> String { "\(currency) \(amount.formatted(.number.precision(.fractionLength(2...6))))" }
    private func read(refresh: Bool) {
        guard !reading, !saving else { return }
        reading = true
        store.readProviderBilling(providerID, refresh: refresh) { result in
            guard active else { return }
            reading = false
            switch result {
            case .success(let value):
                status = value; problem = nil
                if !loaded {
                    if let config = value.configuration {
                        enabled = true; organization = config.organizationId
                        switch config.scope {
                        case .organization: scope = 0
                        case .project(let id), .workspace(let id): scope = 1; scopeID = id
                        case .apiKey(let id): scope = 2; scopeID = id
                        }
                    }
                    loaded = true
                }
            case .failure(let error): problem = error.localizedDescription
            }
        }
    }
    private func save() {
        let selected: BillingScope = scope == 0 ? .organization : scope == 2 ? .apiKey(id: scopeID.trimmingCharacters(in: .whitespacesAndNewlines))
            : service == .openAi ? .project(id: scopeID.trimmingCharacters(in: .whitespacesAndNewlines)) : .workspace(id: scopeID.trimmingCharacters(in: .whitespacesAndNewlines))
        let configuration = enabled ? BillingConfiguration(service: service, organizationId: organization.trimmingCharacters(in: .whitespacesAndNewlines), scope: selected) : nil
        saving = true
        store.saveProviderBilling(providerID, configuration: configuration, authenticationEdit: key.isEmpty ? .keep : .setApiKey(value: key), confirmed: confirmed) { result in
            guard active else { return }
            saving = false
            switch result {
            case .success(let value): status = value; key = ""; problem = nil; confirmed = false
            case .failure(let error): problem = error.localizedDescription
            }
        }
    }
}
