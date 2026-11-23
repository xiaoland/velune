#if VELUNE_DEV_CHECKS
import Foundation
import VeluneBindings

extension DeviceStore {
    /// A background-only activity record must not override core submission admission.
    static func checkBackgroundSubmissionAdmission() {
        let home = FileManager.default.temporaryDirectory.appendingPathComponent("velune-background-admission-\(UUID())", isDirectory: true)
        let transport = Transport(stateDirectory: home, telemetry: .failure(TransportError.unavailable), remote: {
            throw TransportError.rejected("synthetic background admission has no runtime")
        })
        let device = DeviceStore(transport: transport)
        // Keep production queue admission while supplying an in-memory input
        // authority. No runtime or composer transport is started by this check.
        device.prepareBackgroundAdmissionFixture()
        var idle = device.previewSnapshots["sample-0"]!
        idle.runState = .idle
        idle.actions.canSend = true
        idle.actions.canCancel = false
        idle.followUp = .init()
        device.keepPreviewSnapshot(idle)
        let window = ConversationWindowModel(application: device, kind: .workspace)
        let store = window.focused.store
        var sequence: UInt64 = 0
        func adopt(_ value: ConversationSnapshot) {
            sequence += 1
            let conversation = value.conversation
            store.adopt(SnapshotResult(snapshot: DeliveredSnapshot(
                cursor: .init(generation: 1, sequence: sequence), base: nil,
                messageIds: [], messages: [], revision: sequence,
                conversation: .init(id: conversation.id, title: .native(conversation.title),
                    updatedAtUnixMs: nil, createdAtUnixMs: nil, runtimeId: conversation.runtimeID,
                    cwd: nil, canRename: false, canDelete: false, parentConversationId: nil, nativeReadState: nil),
                contextRuntimeId: value.contextRuntimeID, resourceId: nil, modelRecordKey: nil,
                runState: value.runState, transcriptTurns: [], usage: .init(),
                transcriptItems: [], transcriptOutline: [], transcriptMessageIdentities: [],
                messageIdentityConfirmations: [], pendingInteractions: [], actions: value.actions,
                followUp: value.followUp, subAgents: [], collaborators: [])))
            store.prepareSyntheticComposerTargetForAdmissionCheck()
            let context = window.composerContext(for: window.focused)
            precondition(context.inputIssue == nil,
                         "提交准入检查必须先具备完整有效的输入和执行目标：\(context.inputIssue ?? "")")
        }
        adopt(idle)
        var model = window.composer(for: window.focused)
        model.restore("背景任务运行时的新输入", attachments: [])
        device.setBackgroundAdmissionActivity(running: [idle.conversation.id])
        precondition(store.isRunning(idle.conversation), "后台活动仍应显示在会话列表")
        precondition(!store.isGenerating && store.canSend && !store.canQueue,
                     "后台活动不等于 LLM 正在回答，空闲核心快照应允许直接发送")
        let send = window.focused.submission(using: model)
        precondition(send.action == .send && send.canSubmitDraft, "有正文时必须提供可用的发送按钮")
        device.setBackgroundAdmissionActivity(running: [idle.conversation.id], busy: [idle.conversation.id])
        precondition(!store.canSend && !window.focused.submission(using: model).canSubmitDraft,
                     "新轮次活动先到时必须撤销旧空闲快照的发送资格")
        device.setBackgroundAdmissionActivity(running: [idle.conversation.id], elsewhere: [idle.conversation.id])
        precondition(!store.canSend, "外部活动先到时必须撤销旧空闲快照的发送资格")
        device.setBackgroundAdmissionActivity(running: [idle.conversation.id])
        precondition(store.canSend, "仅后台活动仍应允许直接发送")

        var running = idle
        running.conversation = device.previewSnapshots["sample-1"]!.conversation
        running.contextRuntimeID = running.conversation.runtimeID
        running.runState = .running
        running.actions.canSend = false
        running.actions.canCancel = true
        running.followUp.canQueue = true
        device.keepPreviewSnapshot(running)
        adopt(running)
        model = window.composer(for: window.focused)
        model.restore("正在回答时的新输入", attachments: [])
        let queued = window.focused.submission(using: model)
        precondition(!store.canSend && store.canQueue && queued.action == .queue && queued.canSubmitDraft,
                     "真正的 LLM 运行仍应排队")

        var external = idle
        external.conversation = device.previewSnapshots["sample-2"]!.conversation
        external.contextRuntimeID = external.conversation.runtimeID
        external.runState = .elsewhere
        external.actions.canSend = false
        external.actions.inputAccess = .externalController
        device.keepPreviewSnapshot(external)
        adopt(external)
        model = window.composer(for: window.focused)
        model.restore("外部运行时的新输入", attachments: [])
        precondition(!store.canSend && !store.canQueue && !window.focused.submission(using: model).canSubmitDraft,
                     "核心判定外部运行仍必须禁止提交")
        store.close()
        try? FileManager.default.removeItem(at: home)
        FileHandle.standardError.write(Data("PASS background admission: background body sends, early turn/external activity blocks stale idle, actual turn queues, external snapshot blocks\n".utf8))
    }
}
#endif
