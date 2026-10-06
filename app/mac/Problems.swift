import AppKit
import SwiftUI

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

    static func failure(_ error: Error, source: String = "操作", activityKey: String? = nil) -> AppProblem {
        if let transport = error as? TransportError {
            switch transport {
            case .operation(let operation, let cause):
                return failure(cause, source: source == "操作" ? operationName(operation) : source, activityKey: activityKey)
            case .diagnostic(let kind, let detail, let code, let phase, let operationID):
                return AppProblem(id: UUID(), occurredAt: Date(), source: source, detail: detail, kind: kind, code: code, phase: phase, operationID: operationID, activityKey: activityKey)
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
        case "setTranscriptPresentation": return "保存消息列表设置"
        case "setConversationBrowserGroupLimit": return "保存会话列表设置"
        case "runtimeDiscoveryHints", "discoverRuntimes": return "发现 Agent 运行时"
        case "saveProvider": return "保存 AI 提供商"
        case "deleteProvider": return "删除 AI 提供商"
        case "readProviderAPIKey": return "读取 API key"
        case "saveModelTemplate": return "保存模型模板"
        case "deleteModelTemplate": return "删除模型模板"
        case "fetchPublicModelCatalog": return "拉取公开模型目录"
        case "upsertRuntime": return "保存 Agent 运行时"
        case "deleteRuntime": return "删除 Agent 运行时"
        case "selectRuntime": return "选择 Agent 运行时"
        case "createConversation": return "新建会话"
        case "openConversation": return "打开会话"
        case "renameConversation": return "重命名会话"
        case "deleteConversation": return "删除会话"
        case "snapshot": return "读取会话"
        case "sendTurn": return "发送消息"
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
         [code.map { "代码：\($0)" }, phase.map { "阶段：\($0)" }, operationID.map { "操作编号：\($0)" }].compactMap { $0 }).joined(separator: "\n")
    }
}

struct ProblemsButton: View {
    @ObservedObject var store: AppStore
    @Environment(\.openWindow) private var openWindow
    var body: some View {
        Button { openWindow(id: "problems") } label: {
            HStack(spacing: 5) { Image(systemName: "exclamationmark.triangle"); if !store.problems.isEmpty { Text(store.problems.count, format: .number).monospacedDigit() } }
        }.accessibilityLabel("问题，\(store.problems.count) 项").help("查看问题与诊断详情")
    }
}

struct ProblemsSheetHeading: View {
    let title: String
    let store: AppStore
    var body: some View {
        HStack { Text(title).font(.headline); Spacer(); ProblemsButton(store: store).buttonStyle(.borderless) }
    }
}

struct ProblemsMenuItem: View {
    @ObservedObject var store: AppStore
    @Environment(\.openWindow) private var openWindow
    var body: some View {
        Button("问题…") { openWindow(id: "problems") }.keyboardShortcut("m", modifiers: [.command, .shift])
    }
}

extension View {
    func problemsToolbar(_ store: AppStore) -> some View {
        toolbar { ToolbarItem { ProblemsButton(store: store) } }
    }
}

struct ProblemsView: View {
    @ObservedObject var store: AppStore
    @State private var selection: UUID?
    private var selected: AppProblem? { store.problems.first { $0.id == selection } }
    var body: some View {
        HSplitView { problemList; problemDetail }
        .toolbar {
            ToolbarItem {
                Button("复制诊断", systemImage: "doc.on.doc") {
                    guard let selected else { return }
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(selected.diagnosticText, forType: .string)
                }.disabled(selected == nil)
            }
            ToolbarItem {
                Button("清除所选", systemImage: "trash") { if let selection { store.clearProblem(selection) } }.disabled(selected == nil)
            }
            ToolbarItem { Button("全部清除") { store.clearProblems() }.disabled(store.problems.isEmpty) }
        }
        .onChange(of: store.problems) { _, values in
            if let selection, !values.contains(where: { $0.id == selection }) { self.selection = nil }
        }
    }
    private var problemList: some View {
            List(selection: $selection) {
                ForEach(store.problems.reversed()) { problem in
                    VStack(alignment: .leading, spacing: 4) {
                        Text(problem.source).font(.headline).lineLimit(1)
                        Text(problem.detail).lineLimit(2).foregroundStyle(.secondary)
                        Text(problem.occurredAt, format: .dateTime.hour().minute().second()).font(.caption).foregroundStyle(.secondary)
                    }.padding(.vertical, 3).tag(problem.id)
                }
            }.frame(minWidth: 230, idealWidth: 280)
    }
    private var problemDetail: some View {
            ScrollView {
                if let selected {
                    VStack(alignment: .leading, spacing: 16) {
                        Text(selected.source).font(.headline)
                        Text(selected.detail).textSelection(.enabled)
                        Grid(alignment: .leading, horizontalSpacing: 14, verticalSpacing: 10) {
                            diagnosticRow("时间", selected.occurredAt.formatted())
                            diagnosticRow("类别", selected.kind)
                            if let value = selected.code { diagnosticRow("代码", value) }
                            if let value = selected.phase { diagnosticRow("阶段", value) }
                            if let value = selected.operationID { diagnosticRow("操作编号", value) }
                        }.font(.callout).textSelection(.enabled)
                        if selected.activityKey?.hasPrefix("history:") == true {
                            Button("重新读取会话列表") { store.refreshConversations() }.disabled(store.isBusy)
                        }
                    }.frame(maxWidth: .infinity, alignment: .leading).padding(20)
                } else {
                    ContentUnavailableView(store.problems.isEmpty ? "没有问题" : "选择一个问题", systemImage: "checkmark.circle")
                        .frame(maxWidth: .infinity, minHeight: 240)
                }
            }.frame(minWidth: 320, idealWidth: 440)
    }
    private func diagnosticRow(_ label: String, _ value: String) -> some View {
        GridRow { Text(label).foregroundStyle(.secondary); Text(value).frame(maxWidth: .infinity, alignment: .leading) }
    }
}
