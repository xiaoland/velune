import SwiftUI
import AppKit

/// Editing state of the composer in one window. The text view owns the text
/// while editing; each conversation keeps its draft (persisted by the store)
/// and its own undo history.
@MainActor final class ComposerModel: ObservableObject {
    @Published private(set) var isEmpty = true
    @Published private(set) var isFocused = false
    @Published fileprivate(set) var height = ComposerTextView.minimumHeight
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
    }

    /// Clears a sent draft unless it was edited while the send was accepted.
    func clear(submitted: String, conversationID: String?) {
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
    private var nextTurnRuntime: RuntimeInstance? { store.runtimeInstances.first { $0.id == store.nextTurnRuntimeID } }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if store.pendingConversationID == nil, let status = store.conversationManagementStatus {
                HStack { ProgressView().controlSize(.small); Text(status) }.font(.callout).foregroundStyle(.secondary)
            }
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
                if store.executionBusyElsewhere { Text("另一个会话正在运行").font(.callout).foregroundStyle(.secondary).lineLimit(1) }
                else if store.needsModelSelection { Text(store.isUnsentConversation ? "请选择模型" : "此会话的模型不可用").font(.callout).foregroundStyle(.secondary).lineLimit(1) }
                if store.isGenerating {
                    Button("停止", action: store.cancel).disabled(!store.canCancel).help("停止当前生成")
                } else {
                    Button("发送", action: send).buttonStyle(.borderedProminent)
                        .disabled(!store.canSend || model.isEmpty).help("发送消息（⌘ Return）")
                }
            }
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

    private var nextTurnControls: some View {
        HStack(spacing: 8) {
            Menu {
                Picker("Agent 运行时", selection: Binding(get: { store.nextTurnRuntimeID }, set: { if let id = $0 { store.selectNextTurnRuntime(id) } })) {
                    ForEach(store.enabledRuntimeInstances) { runtime in Text(runtime.name).tag(Optional(runtime.id)) }
                }.pickerStyle(.inline)
                Divider()
                SettingsTabLink(tab: .runtimes, title: "Agent 运行时设置…")
            } label: {
                Text(nextTurnRuntime?.name ?? "选择运行时").lineLimit(1).truncationMode(.middle).frame(maxWidth: 140, alignment: .leading)
            }
            .fixedSize()
            .accessibilityLabel("下一轮 Agent 运行时：\(nextTurnRuntime?.name ?? "未选择")")
            .help("下一轮运行时：\(nextTurnRuntime?.name ?? "未选择")")
            .disabled(store.isShuttingDown)
            Menu {
                Picker("模型", selection: Binding(get: { store.nextTurnModelRecordKey }, set: { if let key = $0 { store.selectModel(modelRecordKey: key) } })) {
                    ForEach(store.runtimeCompatibleModels) { model in Text(model.displayName).tag(Optional(model.recordKey)) }
                }.pickerStyle(.inline).disabled(!store.canSwitchModel)
                Divider()
                SettingsTabLink(tab: .providers, title: "AI 提供商设置…")
            } label: {
                Text(store.selectedModelName ?? "选择模型").lineLimit(1).truncationMode(.middle).frame(maxWidth: 200, alignment: .leading)
            }
            .fixedSize()
            .accessibilityLabel("下一轮模型：\(store.selectedModelName ?? "未选择")")
            .help(store.needsModelSelection ? "此会话的模型当前不可用，请选择已配置的模型" : "下一轮模型：\(store.selectedModelName ?? "未选择")")
            .disabled(store.isShuttingDown)
            if store.isUnsentConversation { workingDirectoryMenu }
        }.menuStyle(.button)
    }

    /// Only a conversation that has not been sent yet can still pick where it works.
    private var workingDirectoryMenu: some View {
        let path = store.loadedConversationWorkingDirectory
        let name = path.map { URL(fileURLWithPath: $0).lastPathComponent }
        return Menu {
            Button("选择目录…") { chooseDirectory() }
            if path != nil { Button("使用运行时默认目录") { store.setUnsentWorkingDirectory(nil) } }
        } label: {
            Text(name ?? "工作目录").lineLimit(1).truncationMode(.middle).frame(maxWidth: 160, alignment: .leading)
        }
        .fixedSize()
        .accessibilityLabel("工作目录：\(path ?? "运行时默认目录")")
        .help(path ?? "工作目录：留空时使用运行时的默认目录")
    }

    private func chooseDirectory() {
        let panel = NSOpenPanel()
        panel.title = "选择新会话的工作目录"
        panel.canChooseDirectories = true; panel.canChooseFiles = false; panel.allowsMultipleSelection = false
        if panel.runModal() == .OK, let url = panel.url { store.setUnsentWorkingDirectory(url.standardizedFileURL.path) }
    }
}
