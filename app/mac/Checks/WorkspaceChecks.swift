#if VELUNE_DEV_CHECKS
import AppKit

/// AppKit supplies the coder and later presents the returned window. Only the
/// restoration delegate is exercised here, using windows that stay unordered.
@MainActor func checkWorkspaceWindowRestoration(home: URL) {
    let manager = WorkspaceWindows.shared
    precondition(manager.controllers.isEmpty)
    let directory = home.appendingPathComponent("restoration-core", isDirectory: true)
    let transport = Transport(stateDirectory: directory, resourcesDirectory: Bundle.main.resourceURL!)
    let application = DeviceStore(transport: transport)
    MacNativePersistenceGate.open()
    defer { MacNativePersistenceGate.close(); try! transport.close(stoppingWork: true) }
    manager.start(.init(store: application, previewEmpty: false, previewSettings: false))
    func restore(_ bytes: Data) -> (NSWindow?, Error?) {
        let archive = try! NSKeyedUnarchiver(forReadingFrom: bytes)
        defer { archive.finishDecoding() }
        var calls = 0
        var result: (NSWindow?, Error?) = (nil, nil)
        WorkspaceWindowRestoration.restoreWindow(withIdentifier: .init("restored-window"), state: archive) { window, error in
            calls += 1
            result = (window, error)
        }
        precondition(calls == 1)
        return result
    }
    for (kind, device): (ConversationWindowKind, String?) in [(.workspace, nil), (.single, "fork-peer")] {
        let id = UUID().uuidString
        let saved = ConversationWindowModel(application: application, kind: kind, device: device).savedLayout
        application.windowWorkspaceJournal.save(saved, id: id)
        application.windowWorkspaceJournal.flush()
        let original = manager.restore(kind, id: id, device: device)!
        let controller = manager.controllers.first { $0.window === original }!
        precondition(original.isRestorable && original.restorationClass === WorkspaceWindowRestoration.self
                     && original.delegate === controller, "AppKit must register this window's state writer and restorer")
        let archive = NSKeyedArchiver(requiringSecureCoding: true)
        original.delegate?.window?(original, willEncodeRestorableState: archive)
        archive.finishEncoding()
        original.close()
        precondition(manager.controllers.isEmpty)
        let (restored, error) = restore(archive.encodedData)
        guard let restored, let reopened = manager.controllers.first(where: { $0.window === restored }) else {
            preconditionFailure("窗口恢复回执缺失：\(String(describing: error))")
        }
        precondition(error == nil && restored !== original && reopened.id == id && reopened.kind == kind && reopened.device == device
                     && restored.identifier?.rawValue == "velune.window.\(id)" && !restored.isVisible)
        restored.contentView?.layoutSubtreeIfNeeded()
        RunLoop.main.run(until: Date().addingTimeInterval(0.1))
        // A hidden window need not mount its SwiftUI content on every host.
        // Report that lifecycle boundary; whenever mounted its real onAppear
        // must restore this window's journal, including its pane identity.
        if reopened.model.acceptsWorkspacePersistence {
            precondition(reopened.model.focusedPaneID == saved.focused && reopened.model.savedLayout.panes == saved.panes,
                         "窗口内容启动后必须找回自身 journal 的 pane 和 tab")
        }
        print("Restored hidden \(kind.rawValue) content mounted: \(reopened.model.acceptsWorkspacePersistence)")
        let (same, secondError) = restore(archive.encodedData)
        precondition(same === restored && secondError == nil && manager.controllers.count == 1)
        restored.close()
    }
    // Change one field in an otherwise admissible archived record.
    for (id, kind) in [("broken-uuid", "conversation"), (UUID().uuidString, "broken-kind")] {
        let archive = NSKeyedArchiver(requiringSecureCoding: true)
        archive.encode(id as NSString, forKey: "veluneWindowID")
        archive.encode(kind as NSString, forKey: "veluneWindowKind")
        archive.finishEncoding()
        let (window, error) = restore(archive.encodedData)
        let failure = error as NSError?
        precondition(window == nil && failure?.domain == NSCocoaErrorDomain && failure?.code == CocoaError.coderReadCorrupt.rawValue
                     && manager.controllers.isEmpty, "损坏的恢复标识或类型必须拒绝，不能创建窗口")
    }
}
import Combine
import VeluneBindings

extension ConversationTab {
    /// Development check of the send queue without driving the interface:
    /// send a turn, then queue messages while it runs.
    func checkSendQueue(step: Int = 0, attempts: Int = 0) {
        guard attempts < 120 else {
            FileHandle.standardError.write(Data("FAIL send-queue: step \(step) did not reach its required sending state\n".utf8))
            return
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { [self] in
            let texts = ["第一条排队消息", "第二条排队消息：比较长的一段文字，用来看两行截断在窄面板里是否得体，以及悬停时删除按钮的位置。", "第三条"]
            let ready = step == 0 ? store.canSend : store.canQueue
            guard ready, step == 0 ? store.send(text: "TURN_HOLD") : store.queue(text: texts[step - 1]) else {
                return checkSendQueue(step: step, attempts: attempts + 1)
            }
            if step < texts.count { checkSendQueue(step: step + 1) }
        }
    }

    /// Development check of message editing without driving the interface:
    /// open the editor on the conversation's second user message.
    func checkEditMessage(attempts: Int = 0) {
        guard attempts < 120 else {
            FileHandle.standardError.write(Data("FAIL edit-message: no editable second user message appeared\n".utf8))
            return
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { [self] in
            let users = transcript(of: store).filter { $0.role == .user }
            guard users.count > 1, store.editability(of: users[1].id) != nil else { return checkEditMessage(attempts: attempts + 1) }
            edit(messageID: users[1].id)
        }
    }
    private func transcript(of store: ConversationStore) -> [DeliveredMessage] { store.transcript.rows.map(\.message) }
}

extension ConversationWindowModel {
    /// Development check of the list's run actions without driving the
    /// interface: on the first listed conversation, send from the composer
    /// sheet, stop from the list, then continue from the list.
    func checkListActions(step: Int = 0, attempts: Int = 0) {
        guard attempts < 240 else { return Self.checkLog("FAIL list-actions: step \(step) timed out") }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { [self] in
            let next = { (step: Int) in self.checkListActions(step: step) }
            let retry = { self.checkListActions(step: step, attempts: attempts + 1) }
            // One no tab shows, so continuing goes through a tab that does not show.
            guard let id = application.conversations.first(where: { tab(showing: $0.id) == nil })?.id ?? application.conversations.first?.id else { return retry() }
            switch step {
            case 0:
                openComposerSheet(id)
                Self.checkLog("CHECK list-actions: sheet opened"); next(1)
            case 1:
                guard let tab = composerSheetTab, tab.store.canSend, tab.store.nextTurnSettled else { return retry() }
                tab.composer.restore("SHEET_MESSAGE HOLD", attachments: [])
                // Long enough to capture the sheet with its text.
                DispatchQueue.main.asyncAfter(deadline: .now() + 4) { tab.send(); next(2) }
            case 2:
                guard composerSheetTab == nil else { return retry() }
                Self.checkLog("CHECK list-actions: sheet closed after the send was accepted"); next(3)
            case 3:
                guard application.isStoppable(id) else { return retry() }
                application.stop([id])
                Self.checkLog("CHECK list-actions: stop requested from the list"); next(4)
            case 4:
                guard application.resumableConversations[id] != nil, !application.isConversationRunning(id) else { return retry() }
                resumeFromList(id)
                Self.checkLog("CHECK list-actions: continue requested from the list"); next(5)
            case 5:
                guard !hasBackgroundResumeForCheck, application.resumableConversations[id] == nil else { return retry() }
                Self.checkLog("CHECK list-actions: done")
            default: break
            }
        }
    }

    /// Synthetic check of editor-level presentation and the shared conversation flow.
    func checkSideChat(step: Int = 0, attempts: Int = 0, expected: ConversationTab? = nil, original: ConversationTab? = nil) {
        guard attempts < 180 else { return Self.checkLog("CHECK side-chat: step \(step) timed out") }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { [self] in
            let next = { (step: Int, chat: ConversationTab?, source: ConversationTab?) in
                self.checkSideChat(step: step, expected: chat, original: source)
            }
            let retry = { self.checkSideChat(step: step, attempts: attempts + 1, expected: expected, original: original) }
            guard let id = application.conversations.first?.id else { return retry() }
            switch step {
            case 0:
                sideChatPresentation = .split
                openInNewTab()
                let source = focused
                openSideChat(on: id)
                next(1, nil, source)
            case 1:
                guard let chat = sheetTab, chat.store.canSend, !chat.store.isLoading else { return retry() }
                guard focused === original, application.focusedConversation === original?.store,
                      conversationTabs.allSatisfy({ !$0.store.isSideChat }), original?.store.loadedConversationID != id else {
                    return Self.checkLog("CHECK side-chat: list action changed the current editor")
                }
                let sideID = chat.store.loadedConversationID
                chat.store.createConversation()
                guard chat.store.loadedConversationID == sideID else { return Self.checkLog("CHECK side-chat: new conversation replaced the side session") }
                chat.composer.restore("SIDE_QUESTION_FROM_APP", attachments: [])
                chat.send()
                Self.checkLog("CHECK side-chat: list opened sheet without changing editor; shared send")
                next(2, chat, original)
            case 2:
                guard let chat = expected, chat.store.canSend, !chat.store.isLoading,
                      chat.store.transcript.rows.contains(where: { $0.message.role == .assistant }) else { return retry() }
                focusConversation(chat)
                perform(.find)
                guard commandTab === chat, chat.find.isShown, original?.find.isShown == false else {
                    return Self.checkLog("CHECK side-chat: find targeted another editor")
                }
                chat.find.query = "SIDE_REPLY"
                chat.composer.restore("A FOLLOW-UP STILL BEING TYPED", attachments: [])
                Self.checkLog("CHECK side-chat: sheet answered; shared find")
                DispatchQueue.main.asyncAfter(deadline: .now() + 10) { next(3, chat, original) }
            case 3:
                guard let chat = expected else { return }
                presentSideChat(chat, as: .split)
                guard sheetTab == nil, pane(holding: chat.id) != nil,
                      pane(holding: chat.id)?.id != pane(holding: original!.id)?.id,
                      chat.composer.submission == "A FOLLOW-UP STILL BEING TYPED",
                      savedLayout.isWhole, savedLayout.panes.values.allSatisfy({ !$0.tabs.contains(chat.id) }) else {
                    return Self.checkLog("CHECK side-chat: editor split or restoration is wrong")
                }
                Self.checkLog("CHECK side-chat: peer editor split; draft and reading retained; restoration excludes side")
                DispatchQueue.main.asyncAfter(deadline: .now() + 10) { next(4, chat, original) }
            case 4:
                guard let chat = expected else { return }
                // The retained find field may own focus; two composers may not.
                guard !(chat.composer.isFocused && original?.composer.isFocused == true) else {
                    return Self.checkLog("CHECK side-chat: stale composer focus after moving editor: side=\(chat.composer.isFocused), source=\(original?.composer.isFocused ?? false)")
                }
                sideChatPresentation = .sheet
                guard sheetTab === chat, pane(holding: chat.id) == nil, focused === original,
                      chat.composer.submission == "A FOLLOW-UP STILL BEING TYPED", chat.find.query == "SIDE_REPLY" else {
                    return Self.checkLog("CHECK side-chat: sheet move lost state")
                }
                chat.find.isShown = false
                chat.send()
                next(5, chat, original)
            case 5:
                guard let chat = expected, chat.store.canSend, chat.store.acceptedSubmissions == 2 else { return retry() }
                chat.composer.restore("SIDE_Q_HOLD", attachments: [])
                chat.send()
                next(6, chat, original)
            case 6:
                guard let chat = expected, chat.store.canCancel, !chat.store.isLoading else { return retry() }
                focusConversation(chat)
                guard canPerform(.stop), !canPerform(.attach), !canPerform(.chooseModel) else {
                    return Self.checkLog("CHECK side-chat: wrong command capabilities")
                }
                perform(.stop)
                next(7, chat, original)
            case 7:
                guard let chat = expected, chat.store.canSend else { return retry() }
                chat.composer.restore("SIDE_Q_AFTER_STOP", attachments: [])
                chat.send()
                next(8, chat, original)
            case 8:
                guard let chat = expected, chat.store.canSend, chat.store.acceptedSubmissions == 4 else { return retry() }
                chat.composer.restore("SIDE_Q_CLOSE HOLD", attachments: [])
                chat.send()
                next(9, chat, original)
            case 9:
                guard let chat = expected, chat.store.canCancel, !chat.store.isLoading else { return retry() }
                perform(.closeTab)
                guard commandTab === original, !conversationTabs.contains(where: { $0 === chat }) else {
                    return Self.checkLog("CHECK side-chat: close kept command target")
                }
                original?.store.openLinkedConversation(id: id)
                next(10, nil, original)
            case 10:
                guard let source = original, source.store.sideChatTarget != nil else { return retry() }
                sideChatPresentation = .split
                openSideChat(from: source)
                next(11, focused, original)
            case 11:
                guard let chat = expected, chat.store.canSend else { return retry() }
                guard chat.store.isSideChat, chat.sideChatParentID == original?.id else {
                    return Self.checkLog("CHECK side-chat: menu did not open a peer editor")
                }
                original?.store.createConversation()
                next(12, chat, original)
            case 12:
                guard let chat = expected, !conversationTabs.contains(where: { $0 === chat }) else { return retry() }
                guard savedLayout.isWhole else { return Self.checkLog("CHECK side-chat: invalid saved layout") }
                Self.checkLog("CHECK side-chat: shared stop and follow-up; closed while answering; source lifecycle; done")
            default: break
            }
        }
    }

    /// Unbuffered, so a check launched with `open --stderr` shows its progress.
    private static func checkLog(_ line: String) { FileHandle.standardError.write(Data((line + "\n").utf8)) }
}
#endif
