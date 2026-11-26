import AppKit
import Combine
import SwiftUI
import VeluneBindings

struct AppProblem: Identifiable, Sendable, Equatable {
    let id: UUID
    let occurredAt: Date
    let source: String
    let detail: String
    let kind: String
    let code: String?
    let phase: String?
    let operationID: String?
    let activityKey: String?
    var resolvedAt: Date? = nil

    func record(peerID: String?) -> DiagnosticProblem {
        DiagnosticProblem(id: id.uuidString, occurredAtUnixMs: Int64(occurredAt.timeIntervalSince1970 * 1000), source: source, detail: detail, kind: kind,
                          code: code, phase: phase, operationId: operationID, peerId: peerID, activityKey: activityKey,
                          resolvedAtUnixMs: resolvedAt.map { Int64($0.timeIntervalSince1970 * 1000) })
    }
    static func restored(_ value: DiagnosticProblem) -> AppProblem? {
        guard let id = UUID(uuidString: value.id) else { return nil }
        return AppProblem(id: id, occurredAt: Date(timeIntervalSince1970: Double(value.occurredAtUnixMs) / 1000), source: value.source, detail: value.detail, kind: value.kind,
                          code: value.code, phase: value.phase, operationID: value.operationId, activityKey: value.activityKey,
                          resolvedAt: value.resolvedAtUnixMs.map { Date(timeIntervalSince1970: Double($0) / 1000) })
    }

    static func failure(_ error: Error, source: String = "操作", activityKey: String? = nil) -> AppProblem {
        if let transport = error as? TransportError {
            switch transport {
            case .operation(let operation, let cause):
                return failure(cause, source: source == "操作" ? operationName(operation) : source, activityKey: activityKey)
            case .diagnostic(let kind, let detail, let code, let phase, let operationID):
                return AppProblem(id: UUID(), occurredAt: Date(), source: source, detail: detail, kind: kind, code: code, phase: phase, operationID: operationID.isEmpty ? nil : operationID, activityKey: activityKey)
            default: break
            }
        }
        return AppProblem(id: UUID(), occurredAt: Date(), source: source, detail: error.localizedDescription, kind: "local", code: nil, phase: nil, operationID: nil, activityKey: activityKey)
    }

    private static func operationName(_ value: String) -> String {
        switch value {
        case "shutdown": return "关闭应用"
        case "analyticsQuery": return "读取分析记录"
        case "list": return "读取会话与配置"
        case "listResourceTemplates", "readResourceTemplate", "resourceSyncStatus", "unmanagedRuntimeResources", "importRuntimeResource", "resourceDrift", "resourceNativeImport": return "读取资源库"
        case "saveResourceTemplate": return "保存资源"
        case "deleteResourceTemplate": return "删除资源"
        case "adoptRuntimeResource": return "加入资源库"
        case "applyResourceTemplate", "resolveResourceBinding": return "同步资源"
        case "setConversationBrowserPreferences": return "保存会话列表设置"
        case "setOutlinePresentation": return "保存大纲设置"
        case "setStatusLine": return "保存状态栏设置"
        case "setLinkedConversationOpening": return "保存关联 Agent 设置"
        case "setUsageDisplay": return "保存用量显示设置"
        case "setContextUsageDisplay": return "保存上下文显示设置"
        case "setConversationBrowserLimits": return "保存会话列表设置"
        case "organizeConversations": return "整理会话"
        case "setProject": return "编辑项目"
        case "loadMoreConversations": return "加载会话"
        case "showArchivedConversations": return "切换已归档会话"
        case "searchConversations": return "搜索会话"
        case "findConversation": return "打开会话"
        case "runtimeDiscoveryHints", "discoverRuntimes": return "发现 Agent 运行时"
        case "saveProvider": return "保存 AI 提供商"
        case "deleteProvider": return "删除 AI 提供商"
        case "readProviderAPIKey": return "读取 API key"
        case "saveModelTemplate": return "保存模型模板"
        case "deleteModelTemplate": return "删除模型模板"
        case "fetchPublicModelCatalog": return "拉取公开模型目录"
        case "upsertRuntime": return "保存 Agent 运行时"
        case "deleteRuntime": return "删除 Agent 运行时"
        case "runtimeLaunch": return "读取启动参数"
        case "agentInfo": return "读取 Agent 信息"
        case "sessionOptions": return "读取会话选项"
        case "setSessionOption": return "修改会话选项"
        case "selectRuntime": return "选择 Agent 运行时"
        case "createConversation": return "新建会话"
        case "openConversation": return "打开会话"
        case "renameConversation": return "重命名会话"
        case "deleteConversation": return "删除会话"
        case "snapshot": return "读取会话"
        case "sendTurn": return "发送消息"
        case "sendQueue": return "修改发送队列"
        case "steerTurn": return "插入本轮"
        case "editMessage": return "编辑消息"
        case "cancel": return "停止生成"
        case "replyRuntimeInteraction": return "回应运行时请求"
        case "providerImportPreview": return "读取提供商配置"
        case "providerImportApply": return "导入提供商与模型"
        case "authenticationInspect": return "读取登录信息"
        case "authenticationStart": return "开始登录"
        case "authenticationPoll": return "读取登录进度"
        case "authenticationReply": return "回应登录请求"
        case "authenticationCancel": return "取消登录"
        default: return "操作"
        }
    }
    var diagnosticText: String {
        (["时间：\(occurredAt.formatted())", "来源：\(source)", "类别：\(kind)", "详情：\(detail)"] +
         [resolvedAt.map { "已恢复：\($0.formatted())" }, code.map { "代码：\($0)" }, phase.map { "阶段：\($0)" }, operationID.map { "操作编号：\($0)" }].compactMap { $0 }).joined(separator: "\n")
    }
}

struct ProblemsButton: View {
    @ObservedObject var store: DeviceStore
    @ObservedObject private var model: MergedProblemsModel
    @Environment(\.openWindow) private var openWindow
    private var activeCount: Int { model.entries.filter { $0.problem.resolvedAt == nil }.count }
    init(store: DeviceStore) {
        self.store = store
        model = MergedProblemsModel(local: store, hub: DeviceHub.shared)
    }
    var body: some View {
        Button { openWindow(id: "problems") } label: {
            HStack(spacing: 5) { Image(systemName: "exclamationmark.triangle"); if activeCount > 0 { Text(activeCount, format: .number).monospacedDigit() } }
        }.accessibilityLabel("问题，\(activeCount) 项未恢复").help("查看问题与历史诊断")
    }
}

struct ProblemsMenuItem: View {
    @ObservedObject var store: DeviceStore
    @ObservedObject private var shortcuts = KeyboardShortcutSettings.shared
    @Environment(\.openWindow) private var openWindow
    var body: some View {
        Button(ShortcutCommand.problems.title) { openWindow(id: "problems") }.keyboardShortcut(shortcuts[.problems])
    }
}

/// One problem in the merged list: which device it belongs to and the record.
struct MergedProblem: Identifiable {
    let store: DeviceStore
    let device: String
    let problem: AppProblem
    var id: UUID { problem.id }
}

/// The problems window's model: this Mac and every controlled device
/// contribute their records, labeled by device, merged newest last.
@MainActor final class MergedProblemsModel: ObservableObject {
    @Published private(set) var entries: [MergedProblem] = []
    private let local: DeviceStore
    private let hub: DeviceHub?
    private var watches: Set<AnyCancellable> = []
    private var storeWatches: Set<AnyCancellable> = []
    init(local: DeviceStore, hub: DeviceHub?) {
        self.local = local
        self.hub = hub
        hub?.$state.receive(on: RunLoop.main).sink { [weak self] _ in self?.rebuild() }.store(in: &watches)
        rebuild()
    }
    private func rebuild() {
        storeWatches.removeAll()
        let stores = [local] + (hub?.state?.contacts ?? []).filter(\.controlAuthorized).compactMap { hub?.store(for: $0.peerId) }
        let multiple = stores.count > 1
        var merged: [MergedProblem] = []
        for store in stores {
            let label = store.peerID.map { hub?.name(of: $0) ?? $0 } ?? (multiple ? "这台 Mac" : "")
            store.$problems.receive(on: RunLoop.main).sink { [weak self] _ in
                self?.merge(store: store, label: label)
            }.store(in: &storeWatches)
            merged += store.problems.map { MergedProblem(store: store, device: label, problem: $0) }
        }
        entries = merged.sorted { $0.problem.occurredAt < $1.problem.occurredAt }
    }
    private func merge(store: DeviceStore, label: String) {
        entries.removeAll { $0.store === store }
        entries += store.problems.map { MergedProblem(store: store, device: label, problem: $0) }
        entries.sort { $0.problem.occurredAt < $1.problem.occurredAt }
    }
    /// Clear the record in this Mac's journal through its owning store.
    func clear(id: UUID) {
        entries.first { $0.id == id }?.store.clearProblem(id)
    }
    func clearAll() {
        let stores = [local] + (hub?.state?.contacts ?? []).filter(\.controlAuthorized).compactMap { hub?.store(for: $0.peerId) }
        for store in stores { store.clearProblems() }
    }
}

struct ProblemsView: View {
    /// This Mac's store; the controlled devices' stores come from the hub.
    @StateObject private var model: MergedProblemsModel
    @State private var selection: UUID?
    private var problems: [MergedProblem] { model.entries }
    private var selected: MergedProblem? { problems.first { $0.id == selection } }
    init(local: DeviceStore, hub: DeviceHub?) {
        _model = StateObject(wrappedValue: MergedProblemsModel(local: local, hub: hub))
    }
    var body: some View {
        HSplitView { problemList; problemDetail }
        .toolbar(id: "problems") {
            ToolbarItem(id: "copy") {
                Button("复制诊断", systemImage: "doc.on.doc") {
                    guard let selected else { return }
                    NSPasteboard.general.clearContents()
                    let text = selected.device.isEmpty ? selected.problem.diagnosticText : "设备：\(selected.device)\n\(selected.problem.diagnosticText)"
                    NSPasteboard.general.setString(text, forType: .string)
                }.disabled(selected == nil)
            }
            ToolbarItem(id: "clear-selected") {
                Button("清除所选", systemImage: "trash") {
                    guard let selection else { return }
                    model.clear(id: selection)
                }.disabled(selected == nil)
            }
            ToolbarItem(id: "clear-all") { Button("全部清除") { model.clearAll() }.disabled(problems.isEmpty) }
        }
        .onChange(of: problems.map(\.id)) { _, ids in
            if let selection, !ids.contains(selection) { self.selection = nil }
        }
    }
    private var problemList: some View {
            List(selection: $selection) {
                ForEach(problems.reversed()) { entry in
                    VStack(alignment: .leading, spacing: 4) {
                        Label(entry.problem.source, systemImage: entry.problem.resolvedAt == nil ? "exclamationmark.triangle" : "checkmark.circle").uiFont(.headline).lineLimit(1)
                        if !entry.device.isEmpty { Text(entry.device).uiFont(.caption).foregroundStyle(.secondary).lineLimit(1) }
                        Text(entry.problem.detail).lineLimit(2).foregroundStyle(.secondary)
                        Text(entry.problem.occurredAt, format: .dateTime.month().day().hour().minute().second()).uiFont(.caption).foregroundStyle(.secondary)
                    }.padding(.vertical, 3).tag(entry.id)
                }
            }.frame(minWidth: 230, idealWidth: 280)
    }
    private var problemDetail: some View {
            ScrollView {
                if let selected {
                    VStack(alignment: .leading, spacing: 16) {
                        Text(selected.problem.source).uiFont(.headline)
                        Text(selected.problem.detail).textSelection(.enabled)
                        Grid(alignment: .leading, horizontalSpacing: 14, verticalSpacing: 10) {
                            if !selected.device.isEmpty { diagnosticRow("设备", selected.device) }
                            diagnosticRow("时间", selected.problem.occurredAt.formatted())
                            if let at = selected.problem.resolvedAt { diagnosticRow("已恢复", at.formatted()) }
                            diagnosticRow("类别", selected.problem.kind)
                            if let value = selected.problem.code { diagnosticRow("代码", value) }
                            if let value = selected.problem.phase { diagnosticRow("阶段", value) }
                            if let value = selected.problem.operationID { diagnosticRow("操作编号", value) }
                        }.uiFont(.callout).textSelection(.enabled)
                        if selected.problem.activityKey?.hasPrefix("history:") == true {
                            Button("重新读取会话列表") { selected.store.refreshConversations() }.disabled(selected.store.isBusy)
                        }
                    }.frame(maxWidth: .infinity, alignment: .leading).padding(20)
                } else {
                    ContentUnavailableView(problems.isEmpty ? "没有问题" : "选择一个问题", systemImage: "checkmark.circle")
                        .frame(maxWidth: .infinity, minHeight: 240)
                }
            }.frame(minWidth: 320, idealWidth: 440)
    }
    private func diagnosticRow(_ label: String, _ value: String) -> some View {
        GridRow { Text(label).foregroundStyle(.secondary); Text(value).frame(maxWidth: .infinity, alignment: .leading) }
    }
}
