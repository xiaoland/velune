import AppKit
import Combine
import Darwin
import SwiftUI
import VeluneBindings

struct MessageEditRequest: Identifiable {
    let message: DeliveredMessage
    let conversationID: String?
    let inPlace: Bool
    let isLast: Bool
    let revision: String?
    var id: String { message.id }
}

enum MessageEditPresentation: String, CaseIterable, Identifiable {
    case sheet, floating
    static let preferenceKey = "messageEditPresentation"
    var id: Self { self }
    var title: String { self == .sheet ? "随窗口弹出" : "浮动窗口" }
    static var preferred: Self {
        Self(rawValue: UserDefaults.standard.string(forKey: preferenceKey) ?? "") ?? .sheet
    }
}

/// One edit owns its input and target. Its surface never owns a second draft.
@MainActor final class MessageEditingSession: ObservableObject, Identifiable {
    let id = UUID()
    let request: MessageEditRequest
    @Published private(set) var presentation: MessageEditPresentation
    let tab: ConversationTab
    var store: ConversationStore { tab.store }
    var editor: ComposerModel { tab.composer }
    @Published var placement: EditPlacement? {
        didSet {
            if !updatingPlacementDefault { placementWasChosen = true }
            prepareHistoryConfirmation()
        }
    }
    private var placementWasChosen = false
    private var updatingPlacementDefault = false
    private var placementObservation: AnyCancellable?
    @Published private(set) var confirmationRevision: String?
    @Published private var historyRecheckPending = false
    private var historyReadFinished = false
    private var reloadedRevision: String?
    private var acceptedRevision: String?
    @Published private(set) var choices: ConversationChoices
    @Published private(set) var failure: String?
    @Published private(set) var submitting = false
    var onClose: () -> Void = {}
    var hostAnchor: (() -> NSView?)?
    private var closed = false
    private struct OriginalAttachment {
        let block: Int
        let inline: String?
        let deferred: Bool
        let previewPath: String?
    }
    private var originals: [URL: OriginalAttachment] = [:]
    private var attachmentRuntimeID: String?
    private let recordedModel: String?
    @Published private var loadingAttachments: Set<URL> = []
    @Published private var attachmentFailures: [URL: String] = [:]
    private var attachmentsStarted = false

    init(source: ConversationStore, request: MessageEditRequest, presentation: MessageEditPresentation = .preferred) {
        self.request = request
        self.presentation = presentation
        choices = source.messageEditingChoices(request.message.id)
        recordedModel = source.messageEditingRecordedModel(request.message.id)
        placement = request.inPlace ? .inPlace : .fork
        acceptedRevision = request.revision
        let editingStore = ConversationStore(device: source.device)
        let conversationID = request.conversationID
        if let conversationID { editingStore.noteRestoredConversation(conversationID) }
        tab = ConversationTab(store: editingStore, start: {
            guard let conversationID else { return }
            editingStore.start(loadFirstConversation: false, opening: conversationID)
        })
        let text = request.message.blocks.compactMap { if case .text(let text) = $0 { return text }; return nil }.joined(separator: "\n")
        attachmentRuntimeID = source.messageEditingRuntimeID
        // Each original keeps one chip identity through loading, failure and retry.
        // A remote path is never mistaken for a readable path on this Mac.
        let directory = editingStore.attachmentsDirectory.appendingPathComponent("message-edits", isDirectory: true)
            .appendingPathComponent(id.uuidString, isDirectory: true)
        var files: [URL] = []
        for (index, block) in request.message.blocks.enumerated() {
            let name: String
            var inline: String?
            var deferred = false
            var previewPath: String?
            switch block {
            case .file(let path), .image(_, .path(let path)):
                name = URL(fileURLWithPath: path).lastPathComponent
                if source.isPreview { previewPath = path }
            case .image(let mediaType, .data(let payload)):
                let suffix = Self.imageExtension(mediaType)
                name = "原消息图片 \(index + 1).\(suffix)"
                switch payload {
                case .inline(let value): inline = value
                case .deferred: deferred = true
                }
            default: continue
            }
            let url = directory.appendingPathComponent(String(index), isDirectory: true)
                .appendingPathComponent(name.isEmpty ? "原附件" : name)
            files.append(url)
            originals[url] = OriginalAttachment(block: index, inline: inline, deferred: deferred, previewPath: previewPath)
        }
        editor.attach(editingStore, policy: .messageEdit)
        editor.restore(text, attachments: files)
    }

    func start() {
        guard !closed else { return }
        if placementObservation == nil {
            placementObservation = store.objectWillChange.sink { [weak self] _ in
                // Published values are delivered before assignment. Read the
                // resulting eligibility on the next main-actor turn.
                DispatchQueue.main.async {
                    self?.adoptAvailablePlacementDefault()
                    self?.prepareHistoryConfirmation()
                }
            }
        }
        tab.activate(); loadTargetCatalog(); adoptAvailablePlacementDefault()
        guard !attachmentsStarted, !closed else { return }
        attachmentsStarted = true
        for url in editor.attachments { retryAttachment(url) }
    }
    private func adoptAvailablePlacementDefault() {
        guard !closed, !placementWasChosen, inPlaceAvailable, placement != .inPlace else { return }
        // Qualification may arrive after an initial control check offered only
        // fork. Promote an untouched default once; never override a user choice
        // or silently fork after an established in-place default loses access.
        updatingPlacementDefault = true
        placement = .inPlace
        updatingPlacementDefault = false
    }
    private func prepareHistoryConfirmation() {
        guard !closed, historyRecheckPending, historyReadFinished,
              !store.isLoading, !store.isOpeningConversation,
              store.loadedConversationID == request.conversationID else { return }
        let projectedRevision = store.editability(of: request.message.id)?.revision
        if historyChanged, let projectedRevision, targetIssue == nil {
            confirmationRevision = projectedRevision
        } else {
            // A control check can publish transient eligibility before the
            // confirmed baseline becomes available. Release the check only
            // if the same revision or the target's disappearance is verified.
            let unchanged = reloadedRevision == acceptedRevision && projectedRevision == acceptedRevision
            let removed = reloadedRevision == nil && store.message(id: request.message.id) == nil
            guard unchanged || removed else { return }
        }
        historyRecheckPending = false
    }
    private static func imageExtension(_ mediaType: String?) -> String {
        switch mediaType?.lowercased() {
        case "image/jpeg": "jpg"
        case "image/gif": "gif"
        case "image/webp": "webp"
        case "image/bmp": "bmp"
        case "image/heic", "image/heif": "heic"
        case "image/avif": "avif"
        case "image/tiff": "tiff"
        default: "png"
        }
    }
    /// The ordinary attachment chip retries into its existing private URL.
    func retryAttachment(_ url: URL) {
        guard !closed, !submitting, editor.attachments.contains(url),
              let original = originals[url], !loadingAttachments.contains(url) else { return }
        loadingAttachments.insert(url)
        attachmentFailures[url] = nil
        let transport = store.device.transport
        let runtimeID = attachmentRuntimeID
        let conversationID = request.conversationID
        let messageID = request.message.id
        let messageStamp = request.message.stamp
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let result: Result<Data, Error> = Result {
                if let path = original.previewPath { return try Data(contentsOf: URL(fileURLWithPath: path)) }
                var inline = original.inline
                if inline == nil {
                    guard let transport, let runtimeID, let conversationID else {
                        throw TransportError.rejected("原会话连接不可用")
                    }
                    if original.deferred {
                        let messages = try transport.messagePayloads(runtimeID: runtimeID, conversationID: conversationID, messageIDs: [messageID])
                        guard let message = messages.first(where: { $0.id == messageID }), message.stamp == messageStamp else {
                            throw TransportError.rejected("原消息已有更新，请重新打开编辑")
                        }
                        if message.blocks.indices.contains(original.block),
                           case .image(_, .data(.inline(let value))) = message.blocks[original.block] { inline = value }
                    }
                    if inline == nil {
                        return try transport.readAttachment(runtimeID: runtimeID, conversationID: conversationID,
                                                            messageID: messageID, block: UInt32(original.block)).bytes
                    }
                }
                guard let inline, let bytes = Data(base64Encoded: inline), !bytes.isEmpty else {
                    throw TransportError.rejected("原图片数据无法解码")
                }
                return bytes
            }
            // Keep decoding, large writes and permissions off the UI thread. The
            // sibling staging file shares the destination's volume for atomic rename.
            let staging = url.deletingLastPathComponent().appendingPathComponent(".\(UUID().uuidString).partial")
            let prepared: Result<URL, Error> = Result {
                do {
                    let bytes = try result.get()
                    try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true,
                                                            attributes: [.posixPermissions: 0o700])
                    try bytes.write(to: staging, options: .withoutOverwriting)
                    try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: staging.path)
                    return staging
                } catch {
                    try? FileManager.default.removeItem(at: staging)
                    throw error
                }
            }
            DispatchQueue.main.async {
                // Removal and closing own the lifetime. Unpublished staging data is
                // discarded off-thread even when the session has already disappeared.
                defer {
                    DispatchQueue.global(qos: .utility).async { try? FileManager.default.removeItem(at: staging) }
                }
                guard let self, !self.closed else { return }
                self.loadingAttachments.remove(url)
                guard self.editor.attachments.contains(url) else { return }
                do {
                    let ready = try prepared.get()
                    // Only the constant-size publication runs on the main actor,
                    // serialized with close/remove; no bytes are copied here.
                    guard Darwin.rename(ready.path, url.path) == 0 else {
                        throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno))
                    }
                    self.attachmentFailures[url] = nil
                } catch {
                    self.attachmentFailures[url] = error.localizedDescription
                }
                self.objectWillChange.send()
            }
        }
    }
    private var attachmentIssue: String? {
        let active = editor.attachments
        if active.contains(where: { loadingAttachments.contains($0) }) { return "正在保留原附件，请稍候；也可移除该附件。" }
        if let url = active.first(where: { attachmentFailures[$0] != nil }) {
            return "无法保留原附件「\(url.lastPathComponent)」：\(attachmentFailures[url] ?? "读取失败")。点击附件重试，或重新添加后移除原附件；也可明确移除该附件。"
        }
        return active.contains(where: { !FileManager.default.isReadableFile(atPath: $0.path) })
            ? "附件不可用。请点击附件重试，或重新添加后移除原附件；也可明确移除该附件。" : nil
    }
    func setPresentation(_ value: MessageEditPresentation) { presentation = value }
    func loadTargetCatalog() {
        guard store.loadedConversationID == request.conversationID, let runtime = choices.runtimeID else { return }
        store.loadModelCatalog(runtimeID: runtime)
    }
    static func owning(window: NSWindow?) -> MessageEditingSession? {
        guard let window else { return nil }
        return ConversationWindowModel.openWindows.lazy.flatMap(\.conversationTabs)
            .compactMap(\.editing).first { $0.hostAnchor?()?.window === window }
    }
    var next: NextTurn? {
        guard let runtimeID = choices.runtimeID,
              store.device.enabledRuntimeInstances.contains(where: { $0.id == runtimeID }) else { return nil }
        return store.resolve(kept: choices, editingMessageID: request.message.id)
    }
    var historyChanged: Bool {
        guard !store.isOpeningConversation, store.loadedConversationID == request.conversationID,
              let revision = acceptedRevision else { return false }
        return store.editability(of: request.message.id)?.revision != revision
    }
    var historyNotice: String? {
        if let targetIssue { return targetIssue }
        guard historyChanged else { return nil }
        return placement == .inPlace
            ? "历史已有变化。确认后将替换这条消息及其后的当前内容；编辑副本仍保留。"
            : "历史已有变化。确认后将按当前历史在新会话继续；原会话保留。"
    }
    var noticeText: String? {
        if let failure, confirmationRevision == nil || targetIssue != nil { return failure }
        return executionIssue ?? (next?.chosenModelUnavailable == true ? "所选模型不可用，编辑副本已保留；请重新选择后发送。" : nil)
            ?? historyNotice ?? failure
    }
    var noticeHelp: String? {
        guard let failure, let noticeText, noticeText != failure else { return noticeText }
        return noticeText + "\n" + failure
    }
    private var targetIssue: String? {
        guard !store.isPreview, !store.isOpeningConversation, store.loadedConversationID == request.conversationID else { return nil }
        if store.messageEditingRuntimeID != attachmentRuntimeID { return "原消息的会话来源已变化，无法继续提交；编辑副本仍保留。" }
        guard let eligibility = store.editability(of: request.message.id) else {
            return "原消息已删除或不再允许编辑；编辑副本仍保留。"
        }
        guard acceptedRevision != nil, eligibility.revision != nil else {
            return "当前执行端未提供编辑版本，无法安全提交；请重新连接后再打开编辑，当前副本仍保留。"
        }
        if placement == .inPlace, !eligibility.inPlace {
            return "当前无法在原会话替换后续内容，请选择新会话继续；编辑副本仍保留。"
        }
        return nil
    }
    var inPlaceAvailable: Bool {
        if store.isPreview || store.isOpeningConversation { return request.inPlace }
        return store.editability(of: request.message.id)?.inPlace == true
    }
    var needsPlacement: Bool { request.inPlace || inPlaceAvailable }
    private var modelPreparationIssue: String? {
        guard choices.runtimeID == attachmentRuntimeID, recordedModel != nil,
              next?.model == nil, next?.chosenModelUnavailable != true else { return nil }
        if let runtime = choices.runtimeID, case .failed = store.modelCatalogs[runtime] {
            return "无法确认原消息的模型，请重新读取模型或另选后发送；编辑副本仍保留。"
        }
        return "正在确认原消息的模型，请稍候；也可另选模型。"
    }
    var canSubmit: Bool {
        !closed && !submitting && !historyRecheckPending && !store.isLoading && !store.isShuttingDown && attachmentIssue == nil &&
        store.loadedConversationID == request.conversationID && placement != nil && targetIssue == nil && modelPreparationIssue == nil &&
        (!editor.isEmpty || !editor.attachments.isEmpty) && !editor.isComposing && next != nil && next?.chosenModelUnavailable != true &&
        (store.editability(of: request.message.id).map { placement != .inPlace || $0.inPlace } ?? false)
    }
    var submission: ComposerSubmission {
        ComposerSubmission(phase: submitting ? .sending : .idle,
                           hasInput: !editor.isEmpty || !editor.attachments.isEmpty, isComposing: editor.isComposing,
                           canSend: canSubmit, canQueue: false, canResume: false)
    }
    private var executionIssue: String? {
        if let runtime = choices.runtimeID, !store.device.enabledRuntimeInstances.contains(where: { $0.id == runtime }) {
            return "运行时不可用"
        }
        return (placement == .inPlace ? store.inputLockMessage : nil) ?? targetIssue ?? (historyRecheckPending ? "正在核对原会话，请稍候。" : nil) ?? modelPreparationIssue ?? attachmentIssue
    }
    var target: ComposerEditingTarget {
        ComposerEditingTarget(nextTurn: next,
                              issue: executionIssue,
                              enabled: canSubmit,
                              submitTitle: submitting ? "正在发送…" : confirmationRevision != nil && historyChanged ? (placement == .inPlace ? "确认替换并发送" : "确认新建并发送") : placement == .fork ? "新建并发送" : placement == .inPlace ? "替换并发送" : "发送",
                              chooseRuntime: { [weak self] in self?.chooseRuntime($0) },
                              chooseModel: { [weak self] value in self?.updateChoice { $0.model = value } },
                              chooseEffort: { [weak self] value in self?.updateChoice { $0.effort = value } },
                              choosePermissionMode: { [weak self] value in self?.updateChoice { $0.permissionMode = value } },
                              editable: !closed && !submitting,
                              cancel: { [weak self] in self?.cancel() }, canCancel: !submitting,
                              retryAttachment: { [weak self] in self?.retryAttachment($0) },
                              recordedModelName: choices.runtimeID == attachmentRuntimeID && next?.model == nil ? recordedModel : nil)
    }
    private func chooseRuntime(_ runtime: String) {
        guard !closed, !submitting else { return }
        choices.runtimeID = runtime
        store.loadModelCatalog(runtimeID: runtime)
    }
    private func updateChoice(_ change: (inout TurnChoice) -> Void) {
        guard !closed, !submitting, let runtime = choices.runtimeID else { return }
        var choice = choices.choices[runtime] ?? TurnChoice()
        change(&choice)
        choices.choices[runtime] = choice
    }
    func send(in window: ConversationWindowModel) {
        guard canSubmit, let placement else { return }
        if historyChanged {
            guard let revision = store.editability(of: request.message.id)?.revision else { return }
            guard confirmationRevision == revision else {
                confirmationRevision = revision
                failure = nil
                return
            }
            acceptedRevision = revision
        }
        guard let acceptedRevision else { return }
        confirmationRevision = nil
        failure = nil
        submitting = true
        store.editMessage(request.message.id, placement: placement, text: editor.submission,
                          attachments: editor.attachments.map(\.path), choices: choices, expectedRevision: acceptedRevision,
                          forked: { [weak self, weak window] result in
                              guard let self, !self.closed, let window,
                                    ConversationWindowModel.openWindows.contains(where: { $0 === window }) else { return }
                              window.openFork(adopting: result, device: self.store.device)
                          }, accepted: { [weak self] in self?.cancel() },
                          failed: { [weak self] detail, code in
                              guard let self, !self.closed else { return }
                              self.submitting = false
                              self.failure = detail
                              self.confirmationRevision = nil
                              guard code == "edit_history_changed" ||
                                    detail == "会话已在编辑期间改变，请重新打开编辑后再发送" else { return }
                              self.historyRecheckPending = true
                              self.historyReadFinished = false
                              self.reloadedRevision = nil
                              self.store.refreshMessageEditingState(self.request.message.id) { [weak self] issue, revision in
                                  guard let self, !self.closed else { return }
                                  if let issue {
                                      self.failure = detail + "\n重新核对原会话失败：" + issue
                                      self.historyRecheckPending = false
                                      return
                                  }
                                  self.reloadedRevision = revision
                                  self.historyReadFinished = true
                                  self.prepareHistoryConfirmation()
                              }
                          })
    }
    func cancel() {
        guard !closed else { return }
        closed = true
        placementObservation = nil
        loadingAttachments.removeAll()
        tab.close()
        onClose()
    }
    func copyFailure() {
        guard let failure else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(failure, forType: .string)
    }
    func copyEdit() {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(editor.submission, forType: .string)
    }
    func recover(to source: ConversationTab, in window: ConversationWindowModel) {
        guard attachmentIssue == nil else {
            failure = "原附件尚未保留。请先重试、重新添加或移除不可用附件，再放回输入区。"; return
        }
        guard source.store.loadedConversationID == request.conversationID else {
            failure = "原会话已切换，编辑内容仍保留在这里。"; return
        }
        let input = window.composer(for: source)
        guard input.submission.isEmpty, input.attachments.isEmpty, !input.isComposing,
              source.store.restoreMessageEditingChoices(choices) else {
            failure = "当前输入区已有内容或尚未就绪，编辑内容仍保留在这里。"; return
        }
        input.restore(editor.submission, attachments: editor.attachments)
        cancel()
    }
}

/// Both hosts contain the real composer, with its ordinary typography and controls.
struct MessageEditorView: View {
    @ObservedObject var session: MessageEditingSession
    @ObservedObject private var store: ConversationStore
    @ObservedObject private var editor: ComposerModel
    @ObservedObject var window: ConversationWindowModel
    let source: ConversationTab
    var floating = false
    init(session: MessageEditingSession, window: ConversationWindowModel, source: ConversationTab, floating: Bool = false) {
        self.session = session; self.window = window; self.source = source; self.floating = floating
        _store = ObservedObject(wrappedValue: session.store)
        _editor = ObservedObject(wrappedValue: session.editor)
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                if !floating { Text("编辑消息").uiFont(.headline) }
                Spacer(minLength: 12)
                Button {
                    source.changeMessageEditorPresentation(in: window)
                } label: {
                    Image(systemName: floating ? "rectangle.inset.filled" : "arrow.up.forward.square")
                }
                .buttonStyle(.borderless)
                .help(floating ? "移回会话窗口" : "移到浮动窗口")
                .accessibilityLabel(floating ? "移回会话窗口" : "移到浮动窗口")
                if session.needsPlacement {
                    Picker("继续方式", selection: $session.placement) {
                        Text(session.inPlaceAvailable ? "原会话（替换后续内容）" : "原会话（当前不可替换）")
                            .tag(Optional(EditPlacement.inPlace)).disabled(!session.inPlaceAvailable)
                        Text("新会话（保留原会话）").tag(Optional(EditPlacement.fork))
                    }.labelsHidden().fixedSize().disabled(session.submitting)
                } else {
                    Text("新会话（保留原会话）").uiFont(.callout).foregroundStyle(.secondary)
                }
            }.padding(.horizontal, 16).padding(.top, 16)
            ComposerView(store: store, model: editor, layout: window,
                         send: { session.send(in: window) },
                         steer: {}, submission: { session.submission }, resume: {},
                         fillsHeight: floating, editingTarget: session.target, inputTab: session.tab)
            if session.noticeText != nil {
                HStack(alignment: .top, spacing: 12) {
                    Text(session.noticeText ?? "")
                        .uiFont(.callout).foregroundStyle(.secondary).lineLimit(2).textSelection(.enabled)
                        .help(session.noticeHelp ?? "")
                        .contextMenu {
                            if session.failure != nil { Button("拷贝错误详情", action: session.copyFailure) }
                        }
                    Spacer(minLength: 0)
                    Button("拷贝修改", action: session.copyEdit)
                    if session.failure != nil, store.editability(of: session.request.message.id) == nil {
                        Button("放回输入区") { session.recover(to: source, in: window) }
                            .help("先核对会话结果；只放回空输入区，不会自动发送")
                    }
                }.padding(.horizontal, 16).padding(.bottom, 12)
            }
        }
        .frame(minWidth: 480, idealWidth: 560)
        .background(PaneAnchor { [weak session] anchor in session?.hostAnchor = anchor })
        .onAppear { session.start() }
        .onChange(of: store.loadedConversationID) { _, _ in session.loadTargetCatalog() }
    }
}

/// Created only by the user's edit action. Verification must not show this host.
@MainActor final class MessageEditingPanel: NSPanel, NSWindowDelegate {
    private let session: MessageEditingSession
    private var closing = false
    init(session: MessageEditingSession, window: ConversationWindowModel, source: ConversationTab) {
        self.session = session
        super.init(contentRect: NSRect(x: 0, y: 0, width: 560, height: 260),
                   styleMask: [.titled, .closable, .resizable, .utilityWindow], backing: .buffered, defer: false)
        title = "编辑消息"
        isFloatingPanel = true
        hidesOnDeactivate = true
        becomesKeyOnlyIfNeeded = false
        collectionBehavior = [.fullScreenAuxiliary]
        isReleasedWhenClosed = false
        isRestorable = false
        contentMinSize = NSSize(width: 480, height: 180)
        contentView = NSHostingView(rootView: MessageEditorView(session: session, window: window, source: source, floating: true).applicationAppearance())
        delegate = self
        if let parent = window.window { setFrameOrigin(NSPoint(x: parent.frame.midX - frame.width / 2, y: parent.frame.midY - frame.height / 2)) }
    }
    override var canBecomeKey: Bool { true }
    func showFromUserAction() {
        guard NSApp.activationPolicy() != .prohibited else { return }
        // This entry is only reached by opening, transferring or explicitly
        // bringing back an editor. A utility panel hides while inactive.
        NSApp.activate()
        makeKeyAndOrderFront(nil)
    }
    func closeForTransfer() { closing = true; delegate = nil; close() }
    func closeEditor() { if !closing { close() } }
    func windowWillClose(_ notification: Notification) { closing = true; session.cancel() }
}

#if VELUNE_DEV_CHECKS
extension MessageEditingSession {
    func setVisualCheckSubmitting() { submitting = true }
    func setVisualCheckFailure(_ detail: String) { failure = detail }
}
#endif
