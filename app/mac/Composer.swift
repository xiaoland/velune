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

    fileprivate func edited(_ value: String) {
        text = value
        updateEmpty()
        if let conversationID { store?.setDraft(value, for: conversationID) }
    }

    fileprivate func focusChanged(_ value: Bool) { if isFocused != value { isFocused = value } }

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
    @ObservedObject var model: ComposerModel
    let placeholder: String
    let submit: () -> Void

    func makeCoordinator() -> Coordinator { Coordinator(model: model) }

    func makeNSView(context: Context) -> NSScrollView {
        let scroll = NSScrollView()
        scroll.drawsBackground = false
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.borderType = .noBorder
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
    static let minimumHeight: CGFloat = 2 * ceil(NSFont.preferredFont(forTextStyle: .body).boundingRectForFont.height) + 2 * inset
    static let maximumHeight: CGFloat = 240
    private static let inset: CGFloat = 8
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
        font = .preferredFont(forTextStyle: .body)
        textColor = .labelColor
        typingAttributes = [.font: NSFont.preferredFont(forTextStyle: .body), .foregroundColor: NSColor.labelColor]
        drawsBackground = false
        isRichText = false
        importsGraphics = false
        allowsUndo = true
        isAutomaticQuoteSubstitutionEnabled = false
        isAutomaticDashSubstitutionEnabled = false
        isAutomaticTextReplacementEnabled = false
        textContainerInset = NSSize(width: 4, height: Self.inset)
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
        NSAttributedString(string: placeholder, attributes: [.font: font ?? .preferredFont(forTextStyle: .body), .foregroundColor: NSColor.placeholderTextColor]).draw(in: rect)
    }
    override func didChangeText() {
        super.didChangeText()
        needsDisplay = true
    }
}

/// The bottom panel: one rounded field holding the editor and its few
/// actions — next-turn runtime and model, then send or stop.
struct ComposerView: View {
    @ObservedObject var store: AppStore
    @ObservedObject var model: ComposerModel
    let send: () -> Void
    private var nextTurnRuntime: RuntimeInstance? { store.runtimeInstances.first { $0.id == store.nextTurnRuntimeID } }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if store.pendingConversationID == nil, let status = store.conversationManagementStatus {
                HStack { ProgressView().controlSize(.small); Text(status) }.font(.callout).foregroundStyle(.secondary)
            }
            VStack(spacing: 0) {
                ComposerEditor(model: model, placeholder: "输入消息，⌘Return 发送", submit: send)
                    .frame(height: model.height)
                HStack(spacing: 8) {
                    nextTurnControls
                    Spacer(minLength: 8)
                    if store.executionBusyElsewhere { Text("另一个会话正在运行").font(.caption).foregroundStyle(.secondary).lineLimit(1) }
                    if store.isGenerating {
                        Button(action: store.cancel) { Image(systemName: "stop.fill") }
                            .buttonStyle(.bordered).buttonBorderShape(.circle)
                            .disabled(!store.canCancel).help("停止生成").accessibilityLabel("停止生成")
                    } else {
                        Button(action: send) { Image(systemName: "arrow.up").fontWeight(.semibold) }
                            .buttonStyle(.borderedProminent).buttonBorderShape(.circle)
                            .disabled(!store.canSend || model.isEmpty)
                            .help("发送消息（⌘ Return）").accessibilityLabel("发送")
                    }
                }
                .padding(.horizontal, 8).padding(.bottom, 8)
                // The row's empty space belongs to the field: clicking it edits.
                .background(Color.clear.contentShape(Rectangle()).onTapGesture { model.focus() })
            }
            .background(RoundedRectangle(cornerRadius: 12).fill(Color(nsColor: .textBackgroundColor)))
            .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(model.isFocused ? Color(nsColor: .keyboardFocusIndicatorColor) : Color(nsColor: .separatorColor), lineWidth: model.isFocused ? 2 : 1))
        }
        .frame(maxWidth: 760).padding(.horizontal, 16).padding(.top, 8).padding(.bottom, 14).frame(maxWidth: .infinity)
        .onAppear {
            model.attach(store); model.show(conversationID: store.loadedConversationID)
            // A new window starts in the composer; later selections keep focus where it is.
            DispatchQueue.main.async { model.focus() }
        }
        .onChange(of: store.loadedConversationID) { _, id in model.show(conversationID: id) }
    }

    private var nextTurnControls: some View {
        HStack(spacing: 8) {
            Menu {
                Picker("Agent 运行时", selection: Binding(get: { store.nextTurnRuntimeID }, set: { if let id = $0 { store.selectNextTurnRuntime(id) } })) {
                    ForEach(store.enabledRuntimeInstances) { runtime in Text(runtime.name).tag(Optional(runtime.id)) }
                }.pickerStyle(.inline)
                Divider()
                SettingsLink { Text("设置…") }
            } label: {
                Label { Text(nextTurnRuntime?.name ?? "选择运行时").lineLimit(1).truncationMode(.middle) } icon: { Image(systemName: "terminal") }
            }
            .frame(maxWidth: 140, alignment: .leading)
            .accessibilityLabel("下一轮 Agent 运行时：\(nextTurnRuntime?.name ?? "未选择")")
            .help("下一轮运行时：\(nextTurnRuntime?.name ?? "未选择")")
            .disabled(store.isShuttingDown)
            Menu {
                Picker("模型", selection: Binding(get: { store.nextTurnModelRecordKey }, set: { if let key = $0 { store.selectModel(modelRecordKey: key) } })) {
                    ForEach(store.runtimeCompatibleModels) { model in Text(model.displayName).tag(Optional(model.recordKey)) }
                }.pickerStyle(.inline).disabled(!store.canSwitchModel)
                Divider()
                SettingsLink { Text("设置…") }
            } label: {
                if store.needsModelSelection {
                    Label { Text("选择模型").lineLimit(1) } icon: { Image(systemName: "exclamationmark.triangle.fill") }
                        .foregroundStyle(.orange)
                } else {
                    Label { Text(store.selectedModelName ?? "选择模型").lineLimit(1).truncationMode(.middle) } icon: { Image(systemName: "cpu") }
                }
            }
            .frame(maxWidth: 180, alignment: .leading)
            .accessibilityLabel("下一轮模型：\(store.selectedModelName ?? "未选择")")
            .help(store.needsModelSelection ? "此会话的模型当前不可用，请选择已配置的模型" : "下一轮模型：\(store.selectedModelName ?? "未选择")")
            .disabled(store.isShuttingDown)
        }.menuStyle(.borderlessButton).controlSize(.small).font(.callout).labelStyle(.titleAndIcon)
    }
}
