import SwiftUI
import AppKit
import QuickLookThumbnailing

/// Editing state of the composer in one window. The text view owns the text
/// while editing; each conversation keeps its draft (persisted by the store)
/// and its own undo history.
@MainActor final class ComposerModel: ObservableObject {
    @Published private(set) var isEmpty = true
    @Published private(set) var isFocused = false
    @Published fileprivate(set) var height = ComposerTextView.minimumHeight
    /// Files attached to the next turn of the shown conversation.
    @Published private(set) var attachments: [URL] = []
    private var attachmentsByConversation: [String: [URL]] = [:]
    private(set) var conversationID: String?
    fileprivate weak var textView: ComposerTextView?
    private var text = ""
    private var undoManagers: [String: UndoManager] = [:]
    private weak var store: AppStore?

    var submission: String { text }
    /// Input-method composition owns Return and ⌘Return until it is committed.
    var isComposing: Bool { textView?.hasMarkedText() ?? false }

    func attach(_ store: AppStore) { self.store = store }

    /// Shows the draft of `conversationID`; the previous one is already saved.
    func show(conversationID: String?) {
        guard conversationID != self.conversationID else { return }
        self.conversationID = conversationID
        replace(with: conversationID.flatMap { store?.draft(for: $0) } ?? "")
        attachments = conversationID.flatMap { attachmentsByConversation[$0] } ?? []
    }

    /// Adds regular files; folders and files already attached are skipped.
    func addAttachments(_ urls: [URL]) {
        let files = urls.map(\.standardizedFileURL).filter { url in
            var directory: ObjCBool = false
            return url.isFileURL && FileManager.default.fileExists(atPath: url.path, isDirectory: &directory) && !directory.boolValue
        }
        let added = files.reduce(into: attachments) { list, url in if !list.contains(url) { list.append(url) } }
        guard added != attachments else { return }
        setAttachments(added)
    }

    func removeAttachment(_ url: URL) { setAttachments(attachments.filter { $0 != url }) }

    /// Keeps a pasted image as a PNG file in the application's attachment
    /// folder, so it can be attached by path like any other file.
    fileprivate func attachPastedImage(_ data: Data) {
        guard let store else { return }
        do {
            let directory = store.attachmentsDirectory
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            let url = directory.appendingPathComponent("粘贴的图片 \(Date().formatted(.iso8601.year().month().day().time(includingFractionalSeconds: false).timeSeparator(.omitted).dateTimeSeparator(.space))) \(UUID().uuidString.prefix(4)).png")
            try data.write(to: url, options: .withoutOverwriting)
            addAttachments([url])
        } catch {
            store.recordProblem(error, source: "粘贴图片")
        }
    }

    private func setAttachments(_ value: [URL]) {
        attachments = value
        if let conversationID { attachmentsByConversation[conversationID] = value.isEmpty ? nil : value }
    }

    /// Clears a sent draft unless it was edited while the send was accepted.
    func clear(submitted: String, attachments sent: [URL], conversationID: String?) {
        if !sent.isEmpty {
            attachmentsByConversation = attachmentsByConversation.filter { $0.value != sent }
            if attachments == sent { attachments = [] }
        }
        guard let conversationID else { return }
        if conversationID == self.conversationID {
            guard text == submitted else { return }
            undoManagers[conversationID] = nil
            replace(with: "")
            store?.setDraft("", for: conversationID)
        } else if store?.draft(for: conversationID) == submitted {
            store?.setDraft("", for: conversationID)
        }
    }

    /// Puts an unsent message back, unless the shown conversation has a new draft.
    func restore(_ value: String, attachments sent: [URL]) {
        guard text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, attachments.isEmpty else { return }
        replace(with: value)
        if let conversationID { store?.setDraft(value, for: conversationID) }
        setAttachments(sent)
    }

    func focus() {
        guard let textView, let window = textView.window else { return }
        window.makeFirstResponder(textView)
    }

    fileprivate func undoManager(for view: NSTextView) -> UndoManager? {
        guard let conversationID else { return view.window?.undoManager }
        if let manager = undoManagers[conversationID] { return manager }
        let manager = UndoManager()
        undoManagers[conversationID] = manager
        return manager
    }

    fileprivate func focusChanged(_ value: Bool) { if isFocused != value { isFocused = value } }

    fileprivate func edited(_ value: String) {
        text = value
        updateEmpty()
        if let conversationID { store?.setDraft(value, for: conversationID) }
    }

    fileprivate func attach(_ view: ComposerTextView) {
        textView = view
        if view.string != text { view.replace(text) }
    }

    private func replace(with value: String) {
        text = value
        updateEmpty()
        textView?.replace(value)
    }

    private func updateEmpty() {
        let empty = text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        if isEmpty != empty { isEmpty = empty }
    }
}

/// Native multi-line input: Return inserts a newline, ⌘Return submits, a
/// placeholder shares the text's typography, and the height follows the
/// content up to a limit, after which it scrolls.
struct ComposerEditor: NSViewRepresentable {
    static let cornerRadius: CGFloat = 6
    @ObservedObject var model: ComposerModel
    let placeholder: String
    let submit: () -> Void

    func makeCoordinator() -> Coordinator { Coordinator(model: model) }

    func makeNSView(context: Context) -> NSScrollView {
        let scroll = ComposerFieldView()
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        let view = ComposerTextView()
        view.delegate = context.coordinator
        view.coordinator = context.coordinator
        scroll.documentView = view
        context.coordinator.view = view
        model.attach(view)
        context.coordinator.update(placeholder: placeholder, submit: submit)
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        context.coordinator.model = model
        context.coordinator.update(placeholder: placeholder, submit: submit)
    }

    @MainActor final class Coordinator: NSObject, NSTextViewDelegate {
        var model: ComposerModel
        weak var view: ComposerTextView?
        fileprivate var submit: () -> Void = {}
        init(model: ComposerModel) { self.model = model }

        func update(placeholder: String, submit: @escaping () -> Void) {
            self.submit = submit
            if view?.placeholder != placeholder { view?.placeholder = placeholder }
            view?.fitHeight()
        }
        func textDidChange(_ notification: Notification) {
            guard let view else { return }
            model.edited(view.string)
            view.fitHeight()
        }
        func undoManager(for view: NSTextView) -> UndoManager? { model.undoManager(for: view) }
        fileprivate func heightChanged(_ value: CGFloat) {
            // Reported after layout; SwiftUI resizes the editor in the next pass.
            DispatchQueue.main.async { [model] in if model.height != value { model.height = value } }
        }
    }
}

@MainActor final class ComposerTextView: NSTextView {
    /// One typography for typed text, composition text and the placeholder. The
    /// line height is fixed: otherwise it follows whichever fonts the line holds
    /// (including the input method's own for composition text) and the line
    /// moves while typing.
    private static let font = NSFont.preferredFont(forTextStyle: .body)
    private static let lineHeight = ceil(NSLayoutManager().defaultLineHeight(for: font))
    private static let paragraph: NSParagraphStyle = {
        let style = NSMutableParagraphStyle()
        style.minimumLineHeight = lineHeight; style.maximumLineHeight = lineHeight
        return style
    }()
    private static let typography: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: NSColor.labelColor, .paragraphStyle: paragraph]
    static let minimumHeight: CGFloat = 2 * lineHeight + 2 * inset
    static let maximumHeight: CGFloat = 240
    private static let inset: CGFloat = 5
    fileprivate weak var coordinator: ComposerEditor.Coordinator?
    var placeholder = "" { didSet { needsDisplay = true } }

    init() {
        let storage = NSTextStorage()
        let layoutManager = NSLayoutManager()
        storage.addLayoutManager(layoutManager)
        let container = NSTextContainer(size: NSSize(width: 100, height: CGFloat.greatestFiniteMagnitude))
        container.widthTracksTextView = true
        layoutManager.addTextContainer(container)
        super.init(frame: .zero, textContainer: container)
        font = Self.font
        textColor = .labelColor
        defaultParagraphStyle = Self.paragraph
        typingAttributes = Self.typography
        drawsBackground = false
        isRichText = false
        importsGraphics = false
        allowsUndo = true
        // Plain multi-line text sent verbatim: no substitutions, corrections,
        // smart spacing or detected links rewrite what the user typed or pasted.
        isAutomaticQuoteSubstitutionEnabled = false
        isAutomaticDashSubstitutionEnabled = false
        isAutomaticTextReplacementEnabled = false
        isAutomaticSpellingCorrectionEnabled = false
        isAutomaticLinkDetectionEnabled = false
        isAutomaticDataDetectionEnabled = false
        smartInsertDeleteEnabled = false
        textContainerInset = NSSize(width: 3, height: Self.inset)
        isVerticallyResizable = true
        isHorizontallyResizable = false
        minSize = .zero
        maxSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        autoresizingMask = [.width]
        setAccessibilityLabel("消息内容")
    }
    override init(frame frameRect: NSRect, textContainer container: NSTextContainer?) { super.init(frame: frameRect, textContainer: container) }
    required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }

    /// Programmatic replacement: no undo entry and no draft write.
    func replace(_ value: String) {
        guard string != value else { return }
        string = value
        setSelectedRange(NSRange(location: (value as NSString).length, length: 0))
        needsDisplay = true
        fitHeight()
    }

    func fitHeight() {
        guard let layoutManager, let textContainer else { return }
        layoutManager.ensureLayout(for: textContainer)
        let content = layoutManager.usedRect(for: textContainer).height + 2 * Self.inset
        coordinator?.heightChanged(min(max(ceil(content), Self.minimumHeight), Self.maximumHeight))
    }

    /// Rewrapping at a new width changes the content height.
    override func setFrameSize(_ newSize: NSSize) {
        let widthChanged = newSize.width != frame.width
        super.setFrameSize(newSize)
        if widthChanged { fitHeight() }
    }

    /// Dropped or pasted files become attachments rather than path text.
    override var acceptableDragTypes: [NSPasteboard.PasteboardType] { super.acceptableDragTypes + [.fileURL] }
    override func draggingEntered(_ sender: NSDraggingInfo) -> NSDragOperation {
        Self.fileURLs(sender.draggingPasteboard).isEmpty ? super.draggingEntered(sender) : .copy
    }
    override func draggingUpdated(_ sender: NSDraggingInfo) -> NSDragOperation {
        Self.fileURLs(sender.draggingPasteboard).isEmpty ? super.draggingUpdated(sender) : .copy
    }
    override func performDragOperation(_ sender: NSDraggingInfo) -> Bool {
        let urls = Self.fileURLs(sender.draggingPasteboard)
        guard !urls.isEmpty else { return super.performDragOperation(sender) }
        coordinator?.model.addAttachments(urls)
        return true
    }
    override func paste(_ sender: Any?) {
        let board = NSPasteboard.general
        let urls = Self.fileURLs(board)
        if !urls.isEmpty { coordinator?.model.addAttachments(urls); return }
        // An image on its own, such as a screenshot; text with an image is pasted as text.
        if board.availableType(from: [.string]) == nil, let data = Self.pngData(board) {
            coordinator?.model.attachPastedImage(data); return
        }
        super.paste(sender)
    }
    fileprivate static func fileURLs(_ board: NSPasteboard) -> [URL] {
        (board.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) as? [URL]) ?? []
    }
    private static func pngData(_ board: NSPasteboard) -> Data? {
        if let png = board.data(forType: .png) { return png }
        guard let tiff = board.data(forType: .tiff) else { return nil }
        return NSBitmapImageRep(data: tiff)?.representation(using: .png, properties: [:])
    }

    override func keyDown(with event: NSEvent) {
        let modifiers = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        let isReturn = event.keyCode == 36 || event.keyCode == 76
        if isReturn, modifiers == .command, !hasMarkedText() { coordinator?.submit(); return }
        super.keyDown(with: event)
    }

    override func becomeFirstResponder() -> Bool {
        let accepted = super.becomeFirstResponder()
        if accepted { coordinator?.model.focusChanged(true) }
        return accepted
    }
    override func resignFirstResponder() -> Bool {
        let accepted = super.resignFirstResponder()
        if accepted { coordinator?.model.focusChanged(false) }
        return accepted
    }

    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        guard string.isEmpty, !placeholder.isEmpty, let textContainer else { return }
        let origin = textContainerOrigin
        let padding = textContainer.lineFragmentPadding
        let rect = NSRect(x: origin.x + padding, y: origin.y, width: bounds.width - origin.x * 2 - padding * 2, height: bounds.height - origin.y)
        var attributes = Self.typography
        attributes[.foregroundColor] = NSColor.placeholderTextColor
        NSAttributedString(string: placeholder, attributes: attributes).draw(in: rect)
    }
    override func didChangeText() {
        normalizeTypography()
        super.didChangeText()
        needsDisplay = true
    }

    /// Composition text keeps the input method's underline and clause marks
    /// but is set in the editor's own font and line height.
    override func setMarkedText(_ string: Any, selectedRange: NSRange, replacementRange: NSRange) {
        let marked = NSMutableAttributedString(attributedString: (string as? NSAttributedString) ?? NSAttributedString(string: string as? String ?? ""))
        let whole = NSRange(location: 0, length: marked.length)
        marked.addAttributes([.font: Self.font, .paragraphStyle: Self.paragraph], range: whole)
        super.setMarkedText(marked, selectedRange: selectedRange, replacementRange: replacementRange)
    }

    /// Committed text can arrive carrying another font; plain text has one.
    private func normalizeTypography() {
        typingAttributes = Self.typography
        guard !hasMarkedText(), let storage = textStorage, storage.length > 0 else { return }
        let whole = NSRange(location: 0, length: storage.length)
        var uniform = true
        storage.enumerateAttributes(in: whole) { attributes, _, stop in
            if (attributes[.font] as? NSFont) != Self.font || (attributes[.paragraphStyle] as? NSParagraphStyle) != Self.paragraph { uniform = false; stop.pointee = true }
        }
        guard !uniform else { return }
        storage.beginEditing()
        storage.addAttributes(Self.typography, range: whole)
        storage.endEditing()
    }
}

/// The editor's bezel: a standard text-field background and border that
/// follow the system appearance.
@MainActor final class ComposerFieldView: NSScrollView {
    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        borderType = .noBorder
        drawsBackground = false
        wantsLayer = true
        layer?.cornerRadius = ComposerEditor.cornerRadius
        layer?.masksToBounds = true
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }
    override var wantsUpdateLayer: Bool { true }
    override func updateLayer() {
        layer?.backgroundColor = NSColor.textBackgroundColor.cgColor
        layer?.borderColor = NSColor.separatorColor.cgColor
        layer?.borderWidth = 1
    }
}

/// The bottom panel: a native multi-line field, with the next-turn runtime
/// and model and the send or stop button on a row beneath it.
struct ComposerView: View {
    @ObservedObject var store: AppStore
    @ObservedObject var model: ComposerModel
    let send: () -> Void
    @Environment(\.controlActiveState) private var activeState
    @Environment(\.openWindow) private var openWindow
    @State private var enteringRemoteDirectory = false
    @State private var remoteDirectory = ""
    private var nextTurnRuntime: RuntimeInstance? { store.runtimeInstances.first { $0.id == store.nextTurnRuntimeID } }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if store.pendingConversationID == nil, let status = store.conversationManagementStatus {
                HStack { ProgressView().controlSize(.small); Text(status) }.font(.callout).foregroundStyle(.secondary)
            }
            if !model.attachments.isEmpty { attachmentRow }
            ComposerEditor(model: model, placeholder: "输入消息，⌘Return 发送", submit: send)
                .frame(height: model.height)
                // The text view sits in a clipping scroll view, which hides its
                // own ring; this follows the system ring: focus colour, just
                // outside the field, only while the window is key.
                .overlay {
                    if model.isFocused && activeState == .key {
                        RoundedRectangle(cornerRadius: ComposerEditor.cornerRadius + 2)
                            .strokeBorder(Color(nsColor: .keyboardFocusIndicatorColor), lineWidth: 3).padding(-3)
                            .allowsHitTesting(false)
                    }
                }
            HStack(spacing: 8) {
                nextTurnControls
                Spacer(minLength: 8)
                if store.needsModelSelection { Text(store.isUnsentConversation ? "请选择模型" : "此会话的模型不可用").font(.callout).foregroundStyle(.secondary).lineLimit(1) }
                if store.isGenerating {
                    Button("停止", action: store.cancel).disabled(!store.canCancel).help("停止当前生成")
                } else {
                    Button("发送", action: send).buttonStyle(.borderedProminent)
                        .disabled(!store.canSend || (model.isEmpty && model.attachments.isEmpty)).help("发送消息（⌘ Return）")
                }
            }
        }
        .alert("远端工作目录", isPresented: $enteringRemoteDirectory) {
            TextField("绝对路径", text: $remoteDirectory)
            Button("使用") { store.setUnsentWorkingDirectory(remoteDirectory.trimmingCharacters(in: .whitespacesAndNewlines)) }
                .disabled(!remoteDirectory.trimmingCharacters(in: .whitespacesAndNewlines).hasPrefix("/"))
            Button("取消", role: .cancel) {}
        } message: {
            if case .ssh(let host) = store.location(of: store.nextTurnRuntimeID) { Text("\(host) 上的绝对路径；发送时在该机器上检查。") }
        }
        .onDrop(of: [.fileURL], isTargeted: nil) { providers in
            for provider in providers {
                _ = provider.loadObject(ofClass: NSURL.self) { url, _ in
                    guard let url = url as? URL else { return }
                    Task { @MainActor in model.addAttachments([url]) }
                }
            }
            return !providers.isEmpty
        }
        .frame(maxWidth: 760).padding(.horizontal, 16).padding(.top, 10).padding(.bottom, 14).frame(maxWidth: .infinity)
        .onAppear {
            model.attach(store); model.show(conversationID: store.loadedConversationID)
            // A new window starts in the composer; later selections keep focus where it is.
            DispatchQueue.main.async { model.focus() }
        }
        .onChange(of: store.loadedConversationID) { _, id in
            model.show(conversationID: id)
            // A new conversation starts in the composer.
            if store.isUnsentConversation { DispatchQueue.main.async { model.focus() } }
        }
    }

    private var attachmentRow: some View {
        ScrollView(.horizontal) {
            HStack(spacing: 6) {
                ForEach(model.attachments, id: \.self) { url in AttachmentChip(url: url) { model.removeAttachment(url) } }
            }
        }.scrollIndicators(.never)
    }

    private var nextTurnControls: some View {
        HStack(spacing: 10) {
            Button(action: chooseAttachments) { Image(systemName: "paperclip") }
                .buttonStyle(.borderless)
                .accessibilityLabel("添加附件")
                .help("添加附件：图片随消息发送，其他文件以路径告知 Agent")
                .disabled(store.isShuttingDown)
            agentMenu
            if !store.permissionModeOptions.isEmpty || !store.editableSessionOptions.isEmpty { permissionMenu }
            if store.isUnsentConversation { workingDirectoryMenu }
        }
        // Borderless pull-downs in the system's own colours: the label is the
        // current choice, so the row reads as a sentence rather than a toolbar.
        .menuStyle(.button)
        .buttonStyle(.borderless)
    }

    /// Who answers the next turn. The button reads as the current choice;
    /// the menu keeps one short submenu each for the runtime, its model and
    /// the reasoning level, with the settings that belong to each inside it.
    private var agentMenu: some View {
        let runtime = nextTurnRuntime?.name ?? "选择运行时"
        let model = store.selectedModelName ?? "选择模型"
        // Levels show as the runtime receives them.
        let effort = store.reasoningEffortOptions.isEmpty ? nil : store.nextTurnEffort ?? store.currentEffort
        let title = ([runtime, model] + (effort.map { [$0] } ?? [])).joined(separator: " · ")
        return Menu {
            Menu("运行时：\(runtime)") {
                Picker("Agent 运行时", selection: Binding(get: { store.nextTurnRuntimeID }, set: { if let id = $0 { store.selectNextTurnRuntime(id) } })) {
                    ForEach(store.enabledRuntimeInstances) { runtime in Text(runtime.name).tag(Optional(runtime.id)) }
                }.pickerStyle(.inline).labelsHidden()
                Divider()
                if let runtime = nextTurnRuntime { Button("在资源库中查看 \(runtime.name)…") { ResourceLibraryNavigator.shared.show(runtime.id, openWindow) } }
                SettingsTabLink(tab: .runtimes, title: "Agent 运行时设置…")
            }
            Menu("模型：\(model)") {
                modelMenuContent
                Divider()
                Button("重新读取模型", action: store.reloadModelCatalog).disabled(!store.canSwitchModel || store.modelCatalog == .loading)
                SettingsTabLink(tab: .providers, title: "AI 提供商设置…")
            }
            if !store.reasoningEffortOptions.isEmpty {
                Menu("推理强度：\(effort ?? "沿用当前")") {
                    Picker("推理强度", selection: Binding(get: { store.nextTurnEffort }, set: { store.selectEffort($0) })) {
                        Text(store.currentEffort.map { "沿用当前（\($0)）" } ?? "沿用当前").tag(String?.none)
                            .help("不更改：沿用会话当前的推理强度，新会话由运行时决定")
                        ForEach(store.reasoningEffortOptions, id: \.self) { level in Text(level).tag(Optional(level)) }
                    }.pickerStyle(.inline).labelsHidden()
                }
            }
        } label: {
            Text(Self.pullDownTitle(title)).lineLimit(1).truncationMode(.middle).frame(maxWidth: 360, alignment: .leading)
        }
        .fixedSize()
        .accessibilityLabel("下一轮：运行时 \(runtime)，模型 \(model)，推理强度 \(effort ?? "沿用当前")")
        .help(store.needsModelSelection ? "此会话的模型当前不可用，请选择已配置的模型" : "下一轮：\(runtime) · \(model) · 推理强度 \(effort ?? "沿用当前")")
        .disabled(store.isShuttingDown)
    }

    /// The runtime's own models (and its default, when it has one) come
    /// before the models Velune's providers serve through the gateway.
    @ViewBuilder private var modelMenuContent: some View {
        let choices = store.modelChoices
        let runtimeChoices = choices.filter { if case .managed = $0.selection { false } else { true } }
        let managedChoices = choices.filter { if case .managed = $0.selection { true } else { false } }
        let selection = Binding(get: { store.nextTurnModel }, set: { if let value = $0 { store.selectModel(value) } })
        switch store.modelCatalog {
        case .loading: Text("正在读取运行时的模型…")
        case .failed(let detail): Text("无法读取运行时的模型：\(detail)")
        case .ready, nil: EmptyView()
        }
        if !runtimeChoices.isEmpty {
            Picker("运行时模型", selection: selection) {
                ForEach(runtimeChoices) { model in Text(model.displayName).tag(Optional(model.selection)) }
            }.pickerStyle(.inline).disabled(!store.canSwitchModel)
        }
        if !managedChoices.isEmpty {
            Picker("Velune 提供商", selection: selection) {
                ForEach(managedChoices) { model in Text(model.displayName).tag(Optional(model.selection)) }
            }.pickerStyle(.inline).disabled(!store.canSwitchModel)
        }
    }

    @ViewBuilder private func sessionOptionControl(_ option: SessionOption) -> some View {
        switch option.value {
        case .select(let current):
            let selection = Binding<String>(get: { current }, set: { store.setSessionOption(option, value: .select($0)) })
            Picker(option.name, selection: selection) {
                ForEach(Self.choiceGroups(option.choices), id: \.offset) { group in
                    if let name = group.name { Section(name) { choiceItems(group.choices) } } else { choiceItems(group.choices) }
                }
            }.pickerStyle(.inline).help(option.description ?? option.name)
        case .boolean(let current):
            let isOn = Binding<Bool>(get: { current }, set: { store.setSessionOption(option, value: .boolean($0)) })
            Toggle(option.name, isOn: isOn).help(option.description ?? option.name)
        }
    }

    private func choiceItems(_ choices: [SessionOptionChoice]) -> some View {
        ForEach(choices, id: \.value) { choice in Text(choice.name).tag(choice.value).help(choice.description ?? choice.name) }
    }

    /// Choices in the Agent's order, consecutive ones of a group together.
    private static func choiceGroups(_ choices: [SessionOptionChoice]) -> [(offset: Int, name: String?, choices: [SessionOptionChoice])] {
        var groups: [(offset: Int, name: String?, choices: [SessionOptionChoice])] = []
        for choice in choices {
            if let last = groups.last, last.name == choice.group { groups[groups.count - 1].choices.append(choice) }
            else { groups.append((groups.count, choice.group, [choice])) }
        }
        return groups
    }

    /// What the next turn may do: the runtime type's permission modes, in
    /// its own names, and the live session's own options (an ACP Agent's
    /// modes and the like), which apply at once. A mode is the session's
    /// state in the runtime: choosing none keeps the current one.
    private var permissionMenu: some View {
        let mode = store.effectivePermissionMode
        let current = store.currentPermissionMode
        let hasModes = !store.permissionModeOptions.isEmpty
        let title = hasModes ? (mode ?? current)?.label ?? "默认权限" : "会话选项"
        return Menu {
            if hasModes {
                Picker("权限模式", selection: Binding(get: { mode?.id }, set: { store.selectPermissionMode($0) })) {
                    Text(current.map { "沿用当前（\($0.label)）" } ?? "沿用当前").tag(String?.none)
                        .help("不更改：沿用会话当前的权限模式，新会话为运行时自己的默认")
                    ForEach(store.permissionModeOptions) { option in Text(option.label).tag(Optional(option.id)).help(option.detail) }
                }.pickerStyle(.inline)
            }
            if !store.editableSessionOptions.isEmpty {
                Section("会话选项") {
                    ForEach(store.editableSessionOptions) { option in sessionOptionControl(option) }
                }.disabled(!store.canChangeSessionOptions)
            }
        } label: {
            Label(Self.pullDownTitle(title), systemImage: hasModes ? "lock.shield" : "slider.horizontal.3").lineLimit(1)
        }
        .fixedSize()
        .accessibilityLabel(hasModes ? "下一轮权限模式：\(title)" : "会话选项")
        .help(hasModes ? (mode.map { "权限模式：\($0.label)。\($0.detail)" } ?? "权限模式：沿用会话当前的模式\(current.map { "（\($0.label)）" } ?? "")") : "此会话由 Agent 提供的选项，修改立即生效")
        .disabled(store.isShuttingDown)
    }

    /// The system draws a borderless pull-down's chevron right against its
    /// title (and lays a custom label out as title and icon only), so the
    /// title carries a no-break space to keep the two apart.
    private static func pullDownTitle(_ title: String) -> String { title + "\u{00A0}" }

    /// Only a conversation that has not been sent yet can still pick where it works.
    private var workingDirectoryMenu: some View {
        let path = store.loadedConversationWorkingDirectory
        let name = path.map { URL(fileURLWithPath: $0).lastPathComponent }
        let recent = store.recentWorkingDirectories
        return Menu {
            // A remote runtime's directories exist only on its machine.
            if case .ssh(let host) = store.location(of: store.nextTurnRuntimeID) {
                Button("输入 \(host) 上的目录…") { remoteDirectory = path ?? ""; enteringRemoteDirectory = true }
            } else {
                Button("选择目录…") { chooseDirectory() }
            }
            if path != nil { Button("使用运行时默认目录") { store.setUnsentWorkingDirectory(nil) } }
            if !recent.isEmpty {
                Section("最近使用") {
                    ForEach(recent, id: \.self) { directory in
                        Button(directory) { store.setUnsentWorkingDirectory(directory) }
                    }
                }
            }
        } label: {
            Text(Self.pullDownTitle(name ?? "工作目录")).lineLimit(1).truncationMode(.middle).frame(maxWidth: 160, alignment: .leading)
        }
        .fixedSize()
        .accessibilityLabel("工作目录：\(path ?? "运行时默认目录")")
        .help(path ?? "工作目录：留空时使用运行时的默认目录")
    }

    private func chooseAttachments() {
        let panel = NSOpenPanel()
        panel.title = "添加附件"
        panel.canChooseFiles = true; panel.canChooseDirectories = false; panel.allowsMultipleSelection = true
        if panel.runModal() == .OK { model.addAttachments(panel.urls) }
    }

    private func chooseDirectory() {
        let panel = NSOpenPanel()
        panel.title = "选择新会话的工作目录"
        panel.canChooseDirectories = true; panel.canChooseFiles = false; panel.allowsMultipleSelection = false
        if panel.runModal() == .OK, let url = panel.url { store.setUnsentWorkingDirectory(url.standardizedFileURL.path) }
    }
}

/// An attached file above the composer: its thumbnail or icon, its name, and
/// a button to remove it.
private struct AttachmentChip: View {
    let url: URL
    let remove: () -> Void
    @State private var thumbnail: NSImage?
    @Environment(\.displayScale) private var displayScale

    var body: some View {
        HStack(spacing: 6) {
            Image(nsImage: thumbnail ?? NSWorkspace.shared.icon(forFile: url.path))
                .resizable().aspectRatio(contentMode: .fill)
                .frame(width: 28, height: 28).clipShape(RoundedRectangle(cornerRadius: 4))
            Text(url.lastPathComponent).lineLimit(1).truncationMode(.middle).frame(maxWidth: 160, alignment: .leading)
            Button(action: remove) { Image(systemName: "xmark.circle.fill") }
                .buttonStyle(.borderless).foregroundStyle(.secondary)
                .accessibilityLabel("移除附件 \(url.lastPathComponent)").help("移除附件")
        }
        .padding(4).padding(.trailing, 2)
        .background(.quaternary, in: RoundedRectangle(cornerRadius: 6))
        .help(url.path)
        .task(id: url) {
            let request = QLThumbnailGenerator.Request(fileAt: url, size: CGSize(width: 28, height: 28), scale: displayScale, representationTypes: .thumbnail)
            thumbnail = try? await QLThumbnailGenerator.shared.generateBestRepresentation(for: request).nsImage
        }
    }
}
