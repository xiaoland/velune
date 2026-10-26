import SwiftUI

/// Synthetic preview never opens a remote endpoint or reads device state.
struct PreviewConversation: View {
    @StateObject private var model = PreviewData.model()
    @StateObject private var draft = PreviewData.draft()
    @State private var selected: String? = "sample"
    @State private var query = ""
    @State private var compactColumn: NavigationSplitViewColumn = .detail
    @State private var problems = false
    @Environment(\.horizontalSizeClass) private var sizeClass
    @FocusState private var searching: Bool
    var body: some View {
        NavigationSplitView(preferredCompactColumn: $compactColumn) {
            List(selection: $selected) {
                if !draft.draft.isEmpty {
                    Section("草稿") {
                        NavigationLink(value: "draft") { Label(draft.draft, systemImage: "square.and.pencil").lineLimit(2) }
                    }
                }
                Section("Velune") {
                    ForEach(PreviewData.rows.filter { query.isEmpty || $0.conversation.title.localizedCaseInsensitiveContains(query) }, id: \.conversation.id) { listed in
                        NavigationLink(value: listed.conversation.id) {
                            ConversationListRow(listed: listed, runtimeName: "Codex")
                        }
                    }
                }
            }
            .navigationTitle("开发 Mac").navigationBarTitleDisplayMode(.inline)
            .navigationSplitViewColumnWidth(min: 240, ideal: 280, max: 340)
            .searchable(text: $query, prompt: "搜索会话或项目").searchFocused($searching)
            .toolbar {
                ToolbarItem(placement: .primaryAction) { if sizeClass == .compact { Button("新会话", systemImage: "square.and.pencil", action: create) } }
                ToolbarItem(placement: .bottomBar) {
                    Menu("合成预览") { Button("模拟新消息", action: appendMessage) }.font(.caption).foregroundStyle(.secondary)
                }
                ToolbarItem(placement: .bottomBar) { ProblemsButton() }
            }
        } detail: {
            ConversationContent(model: selected == "draft" ? draft : model, runtimes: PreviewData.runtimes, projects: PreviewData.projects, deviceName: "开发 Mac")
                .id(selected == "draft" ? draft.target.key : model.target.key)
                .toolbar {
                    ToolbarItem(placement: .primaryAction) { Button("新会话", systemImage: "square.and.pencil", action: create) }

                }
        }
        .focusedSceneValue(\.workspaceActions, WorkspaceKeyboardActions(create: create, search: { compactColumn = .sidebar; searching = true }, back: selected == "draft" ? { selected = "sample" } : nil))
        .focusedSceneValue(\.showProblems, { problems = true })
        .sheet(isPresented: $problems) { ProblemsView() }
        .onChange(of: selected) { _, value in
            compactColumn = .detail
            if let row = PreviewData.rows.first(where: { $0.conversation.id == value }), var next = model.snapshot {
                next.conversation.title = row.conversation.title; model.merge(next)
            }
        }
        .onChange(of: draft.runtimeID) { _, _ in draft.models = PreviewData.models }
        .onAppear {
            ProblemStore.shared.record("无法启动 Pi：主机上的辅助程序不存在。\n路径：/sample/tools/pi\n操作：读取历史", source: "读取会话历史", context: "开发 Mac · Pi")
        }
    }
    private func create() { selected = "draft"; compactColumn = .detail }
    private func appendMessage() {
        guard var next = model.snapshot, var message = model.message("assistant") else { return }
        message.stamp += 1
        message.blocks.append(.text(text: "新进度：第 \(message.stamp) 次更新。"))
        next.messages = [model.message("user")!, message]
        next.cursor.sequence += 1
        model.merge(next)
    }
}

@MainActor
enum PreviewData {
    static let runtimes: [BindingRuntimeInstance] = [
        .init(enabled: true, id: "sample", name: "Codex", typeId: "codex", gatewayId: "", settings: [:]),
        .init(enabled: true, id: "claude", name: "Claude Code", typeId: "claude", gatewayId: "", settings: [:])
    ]
    static let projects: [BindingConversationProject] = [
        .init(path: "/sample/velune", name: "Velune", folders: [], worktrees: [], latestActivityUnixMs: nil),
        .init(path: "/sample/website", name: "Website", folders: [], worktrees: [], latestActivityUnixMs: nil)
    ]
    static let models: [BindingRuntimeModel] = [.init(selection: .managed(modelRecordKey: "preview-model"), name: "GPT-5", reasoningEfforts: [])]
    static var rows: [BindingListedConversation] {
        ["iPad 控制端", "改进会话搜索与项目导航，让历史任务更容易找回", "梳理多窗口下的草稿恢复", "检查 Markdown 表格与代码阅读", "Agent 协作：复核本轮改动"].enumerated().map { index, title in
            .init(conversation: .init(id: index == 0 ? "sample" : "sample-\(index)", title: title,
                updatedAtUnixMs: Int64(Date().timeIntervalSince1970 * 1000) - Int64(index * 3600000), createdAtUnixMs: nil,
                runtimeId: "sample", cwd: "/sample/velune", canRename: false, canDelete: false, parentConversationId: nil),
                nativeId: "sample-\(index)", agentsUri: nil, pinned: index == 0, unread: index == 1 || index == 4,
                archived: false, project: "/sample/velune", moved: false, failed: false, group: .project(path: "/sample/velune"))
        }
    }
    static func draft() -> ConversationModel {
        let model = ConversationModel(client: RemoteClient(), peer: "synthetic", target: .init(runtime: "sample", cwd: "/sample/velune"))
        model.loading = false; model.models = models
        return model
    }
    static func model() -> ConversationModel {
        let model = ConversationModel(client: RemoteClient(), peer: "synthetic", target: .init(runtime: "sample", cwd: nil))
        let user = BindingMessage(stamp: 1, completed: true, id: "user", role: .user, timestampUnixMs: nil, blocks: [.text(text: "帮我检查 iPad 控制端的构建结果。")])
        var assistant = BindingMessage(stamp: 2, completed: true, id: "assistant", role: .assistant, timestampUnixMs: nil, blocks: [
            .tool(toolId: "build", title: "检查构建", state: .completed, output: .inline(text: "Build succeeded"), linkedConversationId: nil),
            .text(text: "构建完成。**iPad 和 iPhone 使用同一份应用。**\n\n| 项目 | 状态 |\n| --- | --- |\n| 原生界面 | 已接入 |\n| 远程会话 | 使用共享核心 |\n\n```swift\nNavigationSplitView {\n    ConversationList()\n} detail: {\n    ConversationView()\n}\n```\n\n可以继续检查设备配对与会话操作。")])
        if ProcessInfo.processInfo.arguments.contains("--long") {
            assistant.blocks.append(.text(text: (1...24).map { "### 检查项 \($0)\n\n这是用于验证阅读历史时不跳到底部的合成段落。" }.joined(separator: "\n\n") + "\n\n```swift\nlet unfinished = true"))
        }
        let turn = BindingTranscriptTurn(id: "turn", userMessageId: user.id, workMessageIds: [assistant.id], lastMessageId: assistant.id, replyStartBlock: 1, durationMs: 2400, isRunning: false, settings: nil, usage: nil)
        let snapshot = BindingConversationSnapshot(cursor: .init(generation: 1, sequence: 1), base: nil, messageIds: [user.id, assistant.id], revision: 1,
            conversation: .init(id: "sample", title: "iPad 控制端", updatedAtUnixMs: nil, createdAtUnixMs: nil, runtimeId: "sample", cwd: "/sample/velune", canRename: false, canDelete: false, parentConversationId: nil),
            contextRuntimeId: "sample", resourceId: nil, modelRecordKey: nil, runState: .idle, messages: [user, assistant], transcriptTurns: [turn],
            transcriptItems: [.message(id: "user", messageId: user.id), .work(turnId: turn.id), .message(id: "reply:assistant", messageId: assistant.id)],
            transcriptOutline: [.init(id: "user", messageId: "user")], transcriptMessageIdentities: [], messageIdentityConfirmations: [], pendingInteractions: [],
            actions: .init(canSend: true, canCancel: false, canSwitch: false),
            followUp: .init(queue: [], paused: nil, waiting: nil, canStopSubAgents: false, canSteer: false, canContinue: false, editable: []), subAgents: [.init(conversationId: "preview-child", title: "检查构建结果", failed: false)], collaborators: [.init(conversationId: "preview-peer", title: "复核 iPad 布局", runtimeName: "Claude Code", contacted: true, contactedBy: true)])
        model.merge(snapshot)
        model.loading = false
        model.models = models
        #if DEBUG
        // A streamed reply must keep older messages and split work/reply exactly once.
        var delta = snapshot
        delta.base = snapshot.cursor
        delta.messages = []
        model.merge(delta)
        assert(model.messages.count == 2)
        delta.messageIds = [assistant.id]
        model.merge(delta)
        assert(model.messages[user.id] == nil)
        assert(transcriptBlocks(assistant, turn: turn, reply: false).count == 1)
        assert(transcriptBlocks(assistant, turn: turn, reply: false) + transcriptBlocks(assistant, turn: turn, reply: true) == assistant.blocks)
        model.merge(snapshot)
        let draft = draft()
        let identity = draft.target.key
        draft.draft = "保留两行草稿\n第二行"
        draft.selection = models[0].selection
        draft.chooseRuntime("claude")
        assert(draft.selection == nil && draft.draft.contains("第二行"))
        draft.chooseProject("/sample/website")
        draft.chooseRuntime("sample")
        assert(draft.selection == models[0].selection && draft.target.key == identity)
        draft.selection = nil
        let restored = ConversationModel(client: RemoteClient(), peer: "synthetic", target: draft.target)
        restored.restore(try! JSONDecoder().decode(ComposerState.self, from: JSONEncoder().encode(draft.composerState)))
        var withModel = snapshot
        withModel.modelRecordKey = "previous-model"
        restored.merge(withModel)
        assert(restored.selection == nil && restored.draft == draft.draft)
        assert(restored.destination.cwd == "/sample/website")
        #endif
        return model
    }
}
