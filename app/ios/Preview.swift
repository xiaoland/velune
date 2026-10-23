import SwiftUI

/// Synthetic preview never opens a remote endpoint or reads device state.
struct PreviewConversation: View {
    @StateObject private var model = PreviewData.model()
    @State private var draft = ""
    var body: some View {
        NavigationSplitView {
            List {
                Section("Velune") {
                    Label("iPad 控制端", systemImage: "bubble.left.fill")
                    Label("检查构建结果", systemImage: "bubble.left")
                }
            }.navigationTitle("开发 Mac")
        } detail: {
            ScrollView { TranscriptView(model: model).padding().frame(maxWidth: 900).frame(maxWidth: .infinity) }
                .navigationTitle("iPad 控制端")
                .safeAreaInset(edge: .bottom) {
                    HStack { TextField("发送消息", text: $draft); Image(systemName: "arrow.up.circle.fill").font(.title) }
                        .padding().background(.bar)
                }
        }
    }
}

@MainActor
enum PreviewData {
    static func model() -> ConversationModel {
        let model = ConversationModel(client: RemoteClient(), peer: "synthetic", target: .init(runtime: "sample", cwd: nil))
        let user = BindingMessage(stamp: 1, completed: true, id: "user", role: .user, timestampUnixMs: nil, blocks: [.text(text: "帮我检查 iPad 控制端的构建结果。")])
        let assistant = BindingMessage(stamp: 2, completed: true, id: "assistant", role: .assistant, timestampUnixMs: nil, blocks: [
            .tool(toolId: "build", title: "检查构建", state: .completed, output: .inline(text: "Build succeeded"), linkedConversationId: nil),
            .text(text: "构建完成。**iPad 和 iPhone 使用同一份应用。**\n\n| 项目 | 状态 |\n| --- | --- |\n| 原生界面 | 已接入 |\n| 远程会话 | 使用共享核心 |\n\n```swift\nNavigationSplitView {\n    ConversationList()\n} detail: {\n    ConversationView()\n}\n```\n\n可以继续检查设备配对与会话操作。")])
        let turn = BindingTranscriptTurn(id: "turn", userMessageId: user.id, workMessageIds: [assistant.id], lastMessageId: assistant.id, replyStartBlock: 1, durationMs: 2400, isRunning: false, settings: nil, usage: nil)
        let snapshot = BindingConversationSnapshot(cursor: .init(generation: 1, sequence: 1), base: nil, messageIds: [user.id, assistant.id], revision: 1,
            conversation: .init(id: "sample", title: "iPad 控制端", updatedAtUnixMs: nil, createdAtUnixMs: nil, runtimeId: "sample", cwd: nil, canRename: false, canDelete: false, parentConversationId: nil),
            contextRuntimeId: "sample", resourceId: nil, modelRecordKey: nil, runState: .idle, messages: [user, assistant], transcriptTurns: [turn],
            transcriptItems: [.message(id: "user", messageId: user.id), .work(turnId: turn.id), .message(id: "reply:assistant", messageId: assistant.id)],
            transcriptOutline: [], transcriptMessageIdentities: [], messageIdentityConfirmations: [], pendingInteractions: [],
            actions: .init(canSend: true, canCancel: false, canSwitch: false),
            followUp: .init(queue: [], paused: nil, waiting: nil, canStopSubAgents: false, canSteer: false, canContinue: false, editable: []), subAgents: [], collaborators: [])
        model.merge(snapshot)
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
        #endif
        return model
    }
}
