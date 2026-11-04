import SwiftUI
import Combine
import VeluneBindings
import AppKit
import QuickLookThumbnailing

/// Content identity owns only text and persistent attachment references.
struct SavedComposerDraft: Codable, Equatable {
    var text: String
    var attachments: [URL]
}

/// Editing state of the composer in one window. The text view owns the text
/// while editing; each conversation keeps its draft (persisted by the store)
/// and its own undo history.
@MainActor final class ComposerModel: ObservableObject {
    let id: UUID
    let followsConversation: Bool
    init(followsConversation: Bool = true, id: UUID = UUID()) {
        self.id = id
        self.followsConversation = followsConversation
    }
    @Published private(set) var isEmpty = true
    @Published private(set) var isFocused = false
    @Published fileprivate(set) var height = ComposerTextView.minimumHeight
    /// Files attached to the next turn of the shown conversation.
    @Published private(set) var attachments: [URL] = [] { didSet { if attachments != oldValue { draftChanged() } } }
    private var attachmentsByConversation: [String: [URL]] = [:]
    private(set) var conversationID: String?
    private(set) weak var textView: ComposerTextView?
    private var pendingFocus = false
    private var shownBefore = false
    let draftChanges = PassthroughSubject<Void, Never>()
    private(set) var revision: UInt64 = 0
    var hasUnfinishedInput: Bool { !text.isEmpty || !attachments.isEmpty || undoManagers.values.contains { $0.canUndo } }
    private var selection = NSRange(location: 0, length: 0)
    private var scrollOrigin = NSPoint.zero
    private var conversationRevisions: [String: UInt64] = [:]
    private var changingDraft = false
    private var changedDraft = false
    func draftChanged() {
        if changingDraft { changedDraft = true; return }
        revision &+= 1
        if followsConversation, let conversationID { conversationRevisions[conversationID] = revision }
        draftChanges.send(); objectWillChange.send()
    }
    /// Publish complete semantic edits, so other editors never receive a
    /// half-cleared input while its attachments are being updated.
    private func changeDraft(_ edit: () -> Void) {
        changingDraft = true
        edit()
        changingDraft = false
        if changedDraft { changedDraft = false; draftChanged() }
    }
    func draftRevision(for id: String?) -> UInt64 { followsConversation ? id.flatMap { conversationRevisions[$0] } ?? 0 : revision }
    func restoreFailed(_ text: String, attachments: [URL], for id: String?, revision expected: UInt64?) {
        guard let expected, draftRevision(for: id) == expected else { return }
        if followsConversation, let id, conversationID != id {
            guard let store, store.draft(for: id).isEmpty else { return }
            store.setDraft(text, for: id)
            attachmentsByConversation[id] = attachments
        } else { restore(text, attachments: attachments) }
    }
    private(set) var text = "" { didSet { if text != oldValue { draftChanged() } } }
    private var undoManagers: [String: UndoManager] = [:]
    private(set) weak var store: ConversationStore?
    /// The `/` command or `@` path list over the text (see `ComposerCompletion`).
    @Published var completion: ComposerCompletion? { didSet { if completion != oldValue { showCompletion() } } }
    /// Where the list shows: a window of its own above the text.
    private let completionPanel = ComposerCompletionPanel()
    /// The text Escape closed the list for; it stays closed until the text changes.
    var dismissedCompletionText: String?
    var escapeToken: UUID?
    /// The window's commands, for `/`; set by the composer's view.
    var perform: ((ShortcutCommand) -> Void)?
    var canPerform: ((ShortcutCommand) -> Bool)?
    /// A queued message whose editor the send queue is asked to open.
    @Published var queueEditRequest: QueuedMessage?

    var submission: String { text }
    var savedDraft: SavedComposerDraft? {
        followsConversation ? nil : SavedComposerDraft(text: text, attachments: attachments)
    }
    func restoreSavedDraft(_ saved: SavedComposerDraft?) {
        guard !followsConversation, let saved else { return }
        changeDraft {
            replace(with: saved.text)
            setAttachments(saved.attachments)
        }
    }
    func receiveDraft(_ saved: SavedComposerDraft) {
        if saved.text != text { undoManagers.removeAll() }
        restoreSavedDraft(saved)
    }
    /// Input-method composition owns Return and ⌘Return until it is committed.
    @Published private(set) var isComposing = false
    fileprivate func updateComposition() {
        let value = textView?.hasMarkedText() ?? false
        if isComposing != value { isComposing = value }
    }

    func attach(_ store: ConversationStore) { self.store = store }

    /// Shows the draft of `conversationID`; the previous one is already saved.
    func show(conversationID: String?) {
        guard conversationID != self.conversationID else { return }
        self.conversationID = conversationID
        guard followsConversation else { updateCompletion(); return }
        replace(with: conversationID.flatMap { store?.draft(for: $0) } ?? "")
        attachments = conversationID.flatMap { attachmentsByConversation[$0] } ?? []
    }

    /// Adds regular files; folders and files already attached are skipped.
    func addAttachments(_ urls: [URL]) {
        guard store?.allowsAttachments != false else { return }
        let files = urls.map(\.standardizedFileURL).filter { url in
            var directory: ObjCBool = false
            return url.isFileURL && FileManager.default.fileExists(atPath: url.path, isDirectory: &directory) && !directory.boolValue
        }
        let added = files.reduce(into: attachments) { list, url in if !list.contains(url) { list.append(url) } }
        guard added != attachments else { return }
        setAttachments(added)
    }

    func removeAttachment(_ url: URL) { guard store?.draftEditable != false else { return }; setAttachments(attachments.filter { $0 != url }) }

    func chooseAttachments() {
        let panel = NSOpenPanel()
        panel.title = "添加附件"
        panel.canChooseFiles = true; panel.canChooseDirectories = false; panel.allowsMultipleSelection = true
        if panel.runModal() == .OK { addAttachments(panel.urls) }
        focus()
    }

    /// ⌥↑: the queue's last message back into an empty box, to edit and
    /// queue again; with text in the box, its editor instead.
    func takeLastQueued() -> Bool {
        guard let store, let last = store.followUp.queue.last else { return false }
        guard isEmpty, attachments.isEmpty else { queueEditRequest = last; return true }
        store.removeQueued(last.id) { [weak self] in
            self?.restore(last.text, attachments: last.attachments.map { URL(fileURLWithPath: $0) })
        }
        return true
    }

    /// Keeps a pasted image as a PNG file in the application's attachment
    /// folder, so it can be attached by path like any other file.
    fileprivate func attachPastedImage(_ data: Data) {
        guard let store, store.allowsAttachments else { return }
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
        guard attachments != value else { return }
        attachments = value
        if followsConversation, let conversationID { attachmentsByConversation[conversationID] = value.isEmpty ? nil : value }
    }

    /// Clears a sent draft unless it was edited while the send was accepted.
    func clear(submitted: String, attachments sent: [URL], conversationID: String?) {
        if followsConversation, let conversationID, conversationID != self.conversationID {
            if store?.draft(for: conversationID) == submitted { store?.setDraft("", for: conversationID) }
            if attachmentsByConversation[conversationID] == sent { attachmentsByConversation[conversationID] = nil }
            return
        }
        guard text == submitted, attachments == sent else { return }
        if let conversationID, followsConversation { undoManagers[conversationID] = nil }
        else { undoManagers.removeAll() }
        changeDraft { setAttachments([]); replace(with: "") }
        if followsConversation, let conversationID { store?.setDraft("", for: conversationID) }
    }

    /// Shows the shown conversation's draft as it is kept now, after another
    /// composer (the send sheet) changed it; the text being edited here stays.
    func reloadDraft() {
        guard followsConversation, let conversationID, !(textView?.window?.firstResponder === textView), let kept = store?.draft(for: conversationID), kept != text else { return }
        replace(with: kept)
    }

    /// Puts an unsent message back, unless the shown conversation has a new draft.
    func restore(_ value: String, attachments sent: [URL]) {
        guard text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, attachments.isEmpty else { return }
        changeDraft {
            replace(with: value)
            if followsConversation, let conversationID { store?.setDraft(value, for: conversationID) }
            setAttachments(sent)
        }
    }

    func focus() {
        guard let textView, let window = textView.window else { return }
        window.makeFirstResponder(textView)
    }

    /// Typing goes here once a composer shows this model: its tab is being shown.
    func focusWhenShown() {
        pendingFocus = true
        // A move can still have the old container's view attached this turn.
        DispatchQueue.main.async { [weak self] in
            guard let self, let textView else { return }
            shown(textView)
        }
    }

    /// True the first time a composer shows this model.
    fileprivate func firstShown() -> Bool {
        defer { shownBefore = true }
        return !shownBefore
    }

    fileprivate func shown(_ view: ComposerTextView) {
        // Attaching and removing native views happens during SwiftUI layout.
        // Reconcile on the next turn, using only the model's current view.
        DispatchQueue.main.async { [weak self, weak view] in
            guard let self, let view, view === textView else { return }
            if pendingFocus, let window = view.window, window.makeFirstResponder(view) {
                pendingFocus = false
            }
            focusChanged(view.window.map { $0.firstResponder === view } ?? false)
        }
    }

    fileprivate func undoManager(for view: NSTextView) -> UndoManager? {
        guard let conversationID = followsConversation ? conversationID : "independent-composer" else { return view.window?.undoManager }
        if let manager = undoManagers[conversationID] { return manager }
        let manager = UndoManager()
        undoManagers[conversationID] = manager
        return manager
    }

    fileprivate func focusChanged(_ value: Bool) {
        if isFocused != value { isFocused = value }
        // The list belongs to the text being typed; it closes when typing goes elsewhere.
        if !value { setCompletion(nil) }
    }

    private func showCompletion() {
        completionPanel.show(completion, anchor: textView?.enclosingScrollView) { [weak self] index in self?.acceptCompletion(at: index) }
    }

    fileprivate func edited(_ value: String) {
        text = value
        updateEmpty()
        if followsConversation, let conversationID { store?.setDraft(value, for: conversationID) }
        updateCompletion()
    }

    fileprivate func attach(_ view: ComposerTextView) {
        guard textView !== view else { return }
        textView = view
        if view.string != text { view.replace(text) }
        view.setSelectedRange(NSRange(location: min(selection.location, (text as NSString).length), length: min(selection.length, max(0, (text as NSString).length - selection.location))))
        view.enclosingScrollView?.contentView.scroll(to: scrollOrigin)
    }

    fileprivate func detach(_ view: ComposerTextView) {
        guard textView === view else { return }
        selection = view.selectedRange()
        scrollOrigin = view.enclosingScrollView?.contentView.bounds.origin ?? .zero
        textView = nil
        DispatchQueue.main.async { [weak self] in
            guard let self, textView == nil else { return }
            focusChanged(false)
        }
    }

    private func replace(with value: String) {
        guard value != text else { return }
        text = value
        updateEmpty()
        textView?.replace(value)
        updateCompletion()
    }

    private func updateEmpty() {
        let empty = text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        if isEmpty != empty { isEmpty = empty }
    }
}

/// Native multi-line input: Return or ⌘Return submits as the send mode has
/// it, the other key inserts a newline, a placeholder shares the text's
/// typography, and the height follows the content up to a limit, after
/// which it scrolls.
struct ComposerEditor: NSViewRepresentable {
    static let cornerRadius: CGFloat = 6
    @ObservedObject var model: ComposerModel
    let placeholder: String
    var editable: Bool = true
    let submit: () -> Void
    /// ⌘Return (⌥⌘Return): into the running turn, when the runtime takes it.
    var steer: () -> Void = {}
    /// Added to the text's context menu: the composer's places.
    var placeMenu: () -> NSMenuItem? = { nil }

    func makeCoordinator() -> Coordinator { Coordinator(model: model) }

    func makeNSView(context: Context) -> NSScrollView {
        let scroll = ComposerFieldView()
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        let view = ComposerTextView()
        view.isEditable = editable
        view.delegate = context.coordinator
        view.coordinator = context.coordinator
        scroll.documentView = view
        context.coordinator.view = view
        context.coordinator.placeMenu = placeMenu
        model.attach(view)
        context.coordinator.update(placeholder: placeholder, submit: submit, steer: steer)
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        context.coordinator.view?.isEditable = editable
        if !editable, model.completion != nil { model.completion = nil }
        let previous = context.coordinator.model
        context.coordinator.model = model
        if previous !== model, let view = context.coordinator.view {
            previous.detach(view)
            model.attach(view)
            model.shown(view)
        }
        if let view = context.coordinator.view {
            if view.string != model.submission, !view.hasMarkedText() { view.replace(model.submission) }
            if model.textView == nil || view.window?.firstResponder === view { model.attach(view) }
        }
        context.coordinator.placeMenu = placeMenu
        context.coordinator.update(placeholder: placeholder, submit: submit, steer: steer)
    }

    @MainActor final class Coordinator: NSObject, NSTextViewDelegate {
        var model: ComposerModel
        weak var view: ComposerTextView?
        fileprivate var submit: () -> Void = {}
        fileprivate var steer: () -> Void = {}
        var placeMenu: () -> NSMenuItem? = { nil }
        init(model: ComposerModel) { self.model = model }

        func update(placeholder: String, submit: @escaping () -> Void, steer: @escaping () -> Void) {
            self.submit = submit
            self.steer = steer
            if view?.placeholder != placeholder { view?.placeholder = placeholder }
            view?.fitHeight()
        }
        func textDidChange(_ notification: Notification) {
            guard let view else { return }
            model.updateComposition()
            model.edited(view.string)
            view.fitHeight()
        }
        func undoManager(for view: NSTextView) -> UndoManager? { model.undoManager(for: view) }
        /// Moving the caret into or out of an `@` path opens or closes its list.
        func textViewDidChangeSelection(_ notification: Notification) { model.updateComposition(); model.updateCompletion() }
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
    private static var font: NSFont { TranscriptTypography.body }
    private static var lineHeight: CGFloat { ceil(NSLayoutManager().defaultLineHeight(for: font)) }
    private static var paragraph: NSParagraphStyle {
        let style = NSMutableParagraphStyle()
        style.minimumLineHeight = lineHeight; style.maximumLineHeight = lineHeight
        return style
    }
    private static var typography: [NSAttributedString.Key: Any] { [.font: font, .foregroundColor: NSColor.labelColor, .paragraphStyle: paragraph] }
    static var minimumHeight: CGFloat { lineHeight + 2 * inset }
    static let maximumHeight: CGFloat = 240
    private static let inset: CGFloat = 5
    fileprivate weak var coordinator: ComposerEditor.Coordinator?
    var placeholder = "" { didSet { needsDisplay = true } }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        // Out of its window (another tab shown), its list goes too.
        if window == nil { coordinator?.model.setCompletion(nil) }
        coordinator?.model.shown(self)
    }

    init() {
        let storage = NSTextStorage()
        let layoutManager = NSLayoutManager()
        storage.addLayoutManager(layoutManager)
        let container = NSTextContainer(size: NSSize(width: 100, height: CGFloat.greatestFiniteMagnitude))
        container.widthTracksTextView = true
        layoutManager.addTextContainer(container)
        super.init(frame: .zero, textContainer: container)
        NotificationCenter.default.addObserver(self, selector: #selector(appearanceChanged), name: .veluneAppearanceChanged, object: nil)
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
    @objc private func appearanceChanged() {
        guard !hasMarkedText() else { return }
        let undo = undoManager
        undo?.disableUndoRegistration()
        font = Self.font; defaultParagraphStyle = Self.paragraph; typingAttributes = Self.typography
        textStorage?.addAttributes(Self.typography, range: NSRange(location: 0, length: (string as NSString).length))
        undo?.enableUndoRegistration()
        fitHeight()
    }

    /// Rewrapping at a new width changes the content height.
    override func setFrameSize(_ newSize: NSSize) {
        let widthChanged = newSize.width != frame.width
        super.setFrameSize(newSize)
        if widthChanged { fitHeight() }
    }

    /// Dropped or pasted files become attachments rather than path text.
    override var readablePasteboardTypes: [NSPasteboard.PasteboardType] { [.fileURL, .png, .tiff] + super.readablePasteboardTypes }
    override var acceptableDragTypes: [NSPasteboard.PasteboardType] { super.acceptableDragTypes + [.fileURL] }
    override func draggingEntered(_ sender: NSDraggingInfo) -> NSDragOperation {
        guard isEditable else { return [] }
        return Self.fileURLs(sender.draggingPasteboard).isEmpty ? super.draggingEntered(sender) : .copy
    }
    override func draggingUpdated(_ sender: NSDraggingInfo) -> NSDragOperation {
        guard isEditable else { return [] }
        return Self.fileURLs(sender.draggingPasteboard).isEmpty ? super.draggingUpdated(sender) : .copy
    }
    override func performDragOperation(_ sender: NSDraggingInfo) -> Bool {
        guard isEditable else { return false }
        let urls = Self.fileURLs(sender.draggingPasteboard)
        guard !urls.isEmpty else { return super.performDragOperation(sender) }
        coordinator?.model.addAttachments(urls)
        return true
    }
    /// The text's own menu, then the composer's places.
    override func menu(for event: NSEvent) -> NSMenu? {
        let menu = super.menu(for: event)
        if let menu, let places = coordinator?.placeMenu() {
            menu.addItem(.separator())
            menu.addItem(places)
        }
        return menu
    }

    override func readSelection(from board: NSPasteboard, type: NSPasteboard.PasteboardType) -> Bool {
        guard isEditable else { return false }
        if pasteAttachments(from: board) { return true }
        return super.readSelection(from: board, type: type)
    }

    /// Also handles Finder's file URLs and image representations accompanied by text.
    func pasteAttachments(from board: NSPasteboard) -> Bool {
        guard let model = coordinator?.model, model.store?.allowsAttachments == true else { return false }
        let urls = Self.fileURLs(board)
        if !urls.isEmpty { model.addAttachments(urls); return true }
        if let data = Self.pngData(board) {
            model.attachPastedImage(data); return true
        }
        return false
    }
    fileprivate static func fileURLs(_ board: NSPasteboard) -> [URL] {
        (board.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) as? [URL]) ?? []
    }
    private static func pngData(_ board: NSPasteboard) -> Data? {
        if let png = board.data(forType: .png) { return png }
        guard let tiff = board.data(forType: .tiff) else { return nil }
        return NSBitmapImageRep(data: tiff)?.representation(using: .png, properties: [:])
    }

    /// Composition owns every key until it is committed.
    override func keyDown(with event: NSEvent) {
        guard isEditable, !hasMarkedText(), let coordinator else { return super.keyDown(with: event) }
        if coordinator.model.completionKey(event) { return }
        let modifiers = event.modifierFlags.intersection([.command, .shift, .option, .control])
        switch event.keyCode {
        case 36, 76:
            switch (KeyboardShortcutSettings.shared.sendMode, modifiers) {
            case (.returnSends, []), (.commandReturnSends, .command): return coordinator.submit()
            case (.returnSends, .command), (_, [.command, .option]): return coordinator.steer()
            default: break
            }
        case 126 where modifiers == .option:
            if coordinator.model.takeLastQueued() { return }
        default: break
        }
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
    @ObservedObject var store: ConversationStore
    @ObservedObject var model: ComposerModel
    /// Its window: right-clicked outside the text, the composer offers its places.
    let layout: ConversationWindowModel
    let send: () -> Void
    let steer: () -> Void
    let submission: () -> ComposerSubmission
    let resume: () -> Void
    var fillsHeight = false
    var recipient: String? = nil
    @State private var columns = false
    @Environment(\.controlActiveState) private var activeState
    @Environment(\.openWindow) private var openWindow
    @ObservedObject private var shortcuts = KeyboardShortcutSettings.shared
    @ObservedObject private var appearance = AppearancePreferences.shared
    @State private var showingInputAccess = false
    @State private var showingAccessories = false
    @State private var accessoriesHeight: CGFloat = 64
    @State private var availableHeight: CGFloat = .infinity
    /// Keep the choices beside the send button while the proportionally sized
    /// editor has room; their intrinsic measurement is independent of resizing.
    @State private var oneRow = true
    @State private var fit = RowFit()
    @State private var availableWidth: CGFloat = 760
    private final class RowFit {
        /// What the choices take at full length.
        var controlsWidth: CGFloat = 0
    }
    /// The send button and a little of the status line beside the choices.
    private static let sendWidth: CGFloat = 130
    private var editorMaximumWidth: CGFloat {
        appearance.values.composerWidth(available: availableWidth, reserved: columns ? 2 * (wingWidth + 16) : 0)
    }
    private func refit() {
        let sideWidth = wingWidth > 0 ? 2 * (wingWidth + 16) : 0
        let wide = sideWidth > 0 && availableWidth >= 640 + sideWidth
        if columns != wide { columns = wide }
        let value = appearance.values.composerWidth(available: availableWidth, reserved: wide ? sideWidth : 0) >= fit.controlsWidth + Self.sendWidth
        if oneRow != value { oneRow = value }
    }
    private var wingWidth: CGFloat { max(recipient == nil ? 0 : 156, model.attachments.isEmpty ? 0 : 208) }

    private var context: ComposerContext { ComposerContext(tab: layout.conversationTab(for: store), model: model) }
    private var nextTurnRuntime: RuntimeInstance? { store.device.runtimeInstances.first { $0.id == context.nextTurnRuntimeID } }

    private var hasAccessories: Bool { (store.isUnsentConversation && store.creationFailure != nil) || (!columns && !model.attachments.isEmpty) }
    private var compactAccessories: Bool {
        fillsHeight && availableHeight <= ComposerTextView.minimumHeight + 62 + (oneRow ? 0 : 30) + min(accessoriesHeight, 64)
    }
    private var minimumHeight: CGFloat {
        ComposerTextView.minimumHeight + 54 + (oneRow ? 0 : 30) + (!columns && recipient != nil ? 21 : 0)
    }
    private var accessories: some View {
        VStack(alignment: .leading, spacing: 8) {
            if store.isUnsentConversation, let failure = store.creationFailure { Text(failure).uiFont(.callout).foregroundStyle(.red).textSelection(.enabled) }
            if !model.attachments.isEmpty && !columns { attachmentRow }
        }.frame(maxWidth: .infinity, alignment: .leading)
    }

    var body: some View {
        HStack(alignment: .top, spacing: 16) {
            if columns {
                VStack(alignment: .leading, spacing: 4) {
                    if let recipient {
                        Text("发往").uiFont(.caption).foregroundStyle(.secondary)
                        Text(recipient).uiFont(.callout).lineLimit(availableHeight < 110 ? 1 : 3).truncationMode(.middle)
                    }
                }.frame(width: wingWidth, alignment: .leading).padding(.top, 3)
            }
            editorColumn.frame(maxWidth: editorMaximumWidth)
            if columns {
                VStack(alignment: .leading, spacing: 6) {
                    if !model.attachments.isEmpty {
                        Text("附件").uiFont(.caption).foregroundStyle(.secondary)
                        ScrollView { VStack(alignment: .leading, spacing: 6) { attachmentChips }.frame(maxWidth: .infinity, alignment: .leading) }
                    }
                }.frame(width: wingWidth).frame(height: fillsHeight ? nil : max(model.height + 30, 64))
            }
        }
        .frame(maxWidth: columns ? editorMaximumWidth + 2 * (wingWidth + 16) : editorMaximumWidth)
        .contentShape(Rectangle())
        .contextMenu {
            PreservedInputMenu(window: layout, tab: layout.conversationTab(for: store), model: model)
            Divider()
            ToolPlaceSection(window: layout, kind: .composer)
        }
        .background(alignment: .leading) {
            nextTurnControls(compact: false).fixedSize().hidden().id(controlsKey)
                .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { fit.controlsWidth = $0; refit() }
        }
        .onDrop(of: [.fileURL], isTargeted: nil) { providers in
            guard store.allowsAttachments else { return false }
            for provider in providers {
                _ = provider.loadObject(ofClass: NSURL.self) { url, _ in
                    guard let url = url as? URL else { return }
                    Task { @MainActor in model.addAttachments([url]) }
                }
            }
            return !providers.isEmpty
        }
        .padding(.horizontal, 16).padding(.top, 10).padding(.bottom, 14)
        .frame(minWidth: 0, maxWidth: .infinity)
        .onGeometryChange(for: CGFloat.self) { max(0, $0.size.width - 32) } action: { availableWidth = $0; refit() }
        .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { availableHeight = $0 }
        .onChange(of: model.attachments.count) { _, _ in refit() }
        .onChange(of: recipient) { _, _ in refit() }
        .onChange(of: appearance.values) { _, _ in refit() }
        .onChange(of: store.inputLockMessage) { _, _ in showingInputAccess = false }
        .onChange(of: minimumHeight, initial: true) { _, value in
            if fillsHeight, layout.composerMinimumHeight != value { layout.composerMinimumHeight = value }
        }
        .onChange(of: model.isFocused) { _, focused in
            if focused { layout.focusConversation(layout.conversationTab(for: store)) }
        }
        .onChange(of: ObjectIdentifier(store), initial: true) { _, _ in
            model.attach(store); model.show(conversationID: store.loadedConversationID); context.prepare()
            let followsConversation = model.followsConversation
            model.perform = { [weak layout, weak store] command in layout?.perform(command, for: followsConversation ? store : nil) }
            model.canPerform = { [weak layout, weak store] command in layout?.canPerform(command, composerCommand: true, for: followsConversation ? store : nil) ?? false }
            if model.firstShown() { DispatchQueue.main.async { model.focus() } }
        }
        .onChange(of: store.loadedConversationID) { _, id in
            model.show(conversationID: id); context.prepare()
            if store.isUnsentConversation { DispatchQueue.main.async { model.focus() } }
        }
    }

    private var editorColumn: some View {
        VStack(alignment: .leading, spacing: 8) {
            if !columns, let recipient {
                Text("发往：\(recipient)").uiFont(.caption).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
            }
            if hasAccessories && !compactAccessories {
                if fillsHeight {
                    ScrollView {
                        accessories.onGeometryChange(for: CGFloat.self) { $0.size.height } action: { accessoriesHeight = $0 }
                    }.frame(height: min(accessoriesHeight, 64))
                }
                else { accessories }
            }
            ComposerEditor(model: model, placeholder: placeholder, editable: store.draftEditable, submit: send, steer: steer,
                           placeMenu: { [layout] in layout.placeMenuItem(for: .composer) })
                .frame(height: fillsHeight ? nil : model.height)
                .frame(minHeight: fillsHeight ? ComposerTextView.minimumHeight : nil, maxHeight: fillsHeight ? .infinity : nil)
                // The text view sits in a clipping scroll view, which hides its
                // own ring; this follows the system ring: focus colour, just
                // outside the field, only while the window is key.
                .overlay {
                    if store.draftEditable && model.isFocused && activeState == .key {
                        RoundedRectangle(cornerRadius: ComposerEditor.cornerRadius + 2)
                            .strokeBorder(Color(nsColor: .keyboardFocusIndicatorColor), lineWidth: 3).padding(-3)
                            .allowsHitTesting(false)
                    }
                }
            // A narrow pane puts the choices on their own row, then shortens their titles.
            if oneRow {
                HStack(spacing: 8) { nextTurnControls(compact: false); accessoriesButton; statusRow }
            } else {
                HStack { ViewThatFits(in: .horizontal) { nextTurnControls(compact: false); nextTurnControls(compact: true) }; accessoriesButton }
                statusRow
            }
        }
    }

    @ViewBuilder private var accessoriesButton: some View {
        if compactAccessories && hasAccessories {
            Button { showingAccessories = true } label: { Image(systemName: "tray.full") }
                .buttonStyle(.borderless).help("查看附件与输入详情")
                .accessibilityLabel("输入详情")
                .popover(isPresented: $showingAccessories) {
                    ScrollView {
                        accessories.onGeometryChange(for: CGFloat.self) { $0.size.height } action: { accessoriesHeight = $0 }
                            .padding(12)
                    }.frame(width: 440, height: min(max(accessoriesHeight + 24, 64), 320))
                }
        }
    }

    /// The keys as the send mode has them, and what they do now.
    private var placeholder: String {
        if !store.inputSubmissionAllowed { return store.inputAccess.composerPlaceholder }
        let mode = shortcuts.sendMode
        guard store.isGenerating else { return mode == .returnSends ? "输入消息，Return 发送，⇧Return 换行，/ 命令" : "输入消息，⌘Return 发送，/ 命令" }
        if !store.canQueue { return "正在回答；可继续编辑，结束后发送" }
        return store.followUp.canSteer ? "\(mode.sendKey) 排队，\(mode.steerKey) 插入本轮" : "\(mode.sendKey) 排队，本轮结束后发送"
    }

    /// What the choices' titles are made of.
    private var controlsKey: [String] {
        let context = self.context
        return [nextTurnRuntime?.name ?? "", context.selectedModelName ?? "", context.nextTurnEffort ?? context.currentEffort ?? "",
         (context.effectivePermissionMode ?? context.currentPermissionMode)?.label ?? "", "\(context.permissionModeOptions.isEmpty)\(store.editableSessionOptions.isEmpty)",
         store.isUnsentConversation ? store.loadedConversationWorkingDirectory ?? "-" : ""]
    }

    /// The status line and send button, at the trailing end.
    private var statusRow: some View {
        let context = self.context
        return HStack(spacing: 8) {
            Spacer(minLength: 8)
            if let issue = context.inputIssue { Text(issue).uiFont(.callout).foregroundStyle(.secondary).lineLimit(1).help(issue) }
            else if context.chosenModelUnavailable { Text("所选模型已不可用").uiFont(.callout).foregroundStyle(.secondary).lineLimit(1) }
            else { ComposerStatusLine(store: store, context: context).layoutPriority(-1).padding(.trailing, 4) }
            sendButton
        }.frame(minHeight: 22)
    }

    @ViewBuilder private var sendButton: some View {
        let current = submission()
        if store.canCancel {
            Button("停止", action: store.cancel).disabled(store.isLoading).help("停止当前生成")
        }
        if store.inputSubmissionAllowed, current.action == .queue && store.followUp.canSteer {
            Button("插入本轮", action: steer).disabled(!context.canSteer)
                .help("将消息插入正在运行的本轮")
        }
        if store.followUp.canStopSubAgents && !store.isGenerating && !store.followUp.waitingForSubAgents {
            Button("停止子 Agent", action: store.stopSubAgents).disabled(store.isLoading)
        }
        if let message = store.inputLockMessage {
            Button { showingInputAccess = true } label: {
                Label(store.inputAccess.composerTitle, systemImage: "lock.fill")
                    .uiFont(.callout).lineLimit(1)
            }
            .buttonStyle(.borderless).foregroundStyle(.secondary).help(message)
            .popover(isPresented: $showingInputAccess) {
                Text(message).textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
                    .padding(16).frame(width: 300, alignment: .leading)
            }
        } else if current.action != .none {
            Button(current.action.title, action: current.action == .resume ? resume : send)
                .buttonStyle(.borderedProminent).disabled(!current.enabled)
                .help(current.action == .resume ? store.continueHelp : current.action == .queue ? "本轮结束后发送" : "发送消息")
        }
    }

    private var attachmentRow: some View {
        ScrollView(.horizontal) {
            HStack(spacing: 6) { attachmentChips }
        }.scrollIndicators(.never)
    }

    private var attachmentChips: some View {
        ForEach(model.attachments, id: \.self) { url in
            AttachmentChip(url: url, canRemove: store.draftEditable, retry: { store.device.composerInputs.poll(force: true) }) { model.removeAttachment(url) }
        }
    }

    /// `compact` shortens the choices' titles to fit a narrow pane.
    private func nextTurnControls(compact: Bool) -> some View {
        HStack(spacing: 10) {
            if store.allowsAttachments {
                Button(action: model.chooseAttachments) { Image(systemName: "paperclip") }
                    .buttonStyle(.borderless)
                    .accessibilityLabel("添加附件")
                    .help(KeyboardShortcutSettings.shared.help("添加附件：图片随消息发送，其他文件以路径告知 Agent", .attach))
                    .disabled(store.isShuttingDown)
            }
            agentMenu(compact: compact)
            if !context.permissionModeOptions.isEmpty || !store.editableSessionOptions.isEmpty { permissionMenu(compact: compact) }
            if store.isUnsentConversation { workingDirectoryMenu(compact: compact) }
        }
        // Borderless pull-downs in the system's own colours: the label is the
        // current choice, so the row reads as a sentence rather than a toolbar.
        .menuStyle(.button)
        .buttonStyle(.borderless)
    }

    /// Who answers the next turn. The button reads as the current choice;
    /// the menu keeps one short submenu each for the runtime, its model and
    /// the reasoning level, with the settings that belong to each inside it.
    private func agentMenu(compact: Bool) -> some View {
        let context = self.context
        let runtime = nextTurnRuntime?.name ?? "选择运行时"
        let model = context.selectedModelName ?? "选择模型"
        // Levels show as the runtime receives them.
        let effort = context.reasoningEffortOptions.isEmpty ? nil : context.nextTurnEffort ?? context.currentEffort
        // An unchosen level follows the conversation's, or is the runtime's own.
        let unchosen = context.nextTurnHasCurrent ? "沿用当前" : "运行时默认"
        let title = ([runtime, model] + (effort.map { [$0] } ?? [])).joined(separator: " · ")
        return Menu {
            Menu("运行时：\(runtime)") {
                Picker("Agent 运行时", selection: Binding(get: { context.nextTurnRuntimeID }, set: { if let id = $0 { context.selectNextTurnRuntime(id) } })) {
                    ForEach(store.device.enabledRuntimeInstances.filter { !store.isSideChat || store.device.hasSideChats($0.id) }) { runtime in Text(runtime.name).tag(Optional(runtime.id)) }
                }.pickerStyle(.inline).labelsHidden()
                Divider()
                if let runtime = nextTurnRuntime { Button("在资源库中查看 \(runtime.name)…") { ResourceLibraryNavigator.shared.show(runtime.id, openWindow) } }
                SettingsTabLink(tab: .runtimes, title: "Agent 运行时设置…")
            }
            Menu("模型：\(model)") {
                modelMenuContent
                Divider()
                Button("重新读取模型", action: context.reloadModelCatalog).disabled(!context.canSwitchModel || context.modelCatalog == .loading)
                SettingsTabLink(tab: .providers, title: "AI 提供商设置…")
            }
            if !context.reasoningEffortOptions.isEmpty {
                Menu("推理强度：\(effort ?? unchosen)") {
                    Picker("推理强度", selection: Binding(get: { context.nextTurnEffort }, set: { context.selectEffort($0) })) {
                        Text(context.currentEffort.map { "沿用当前（\($0)）" } ?? unchosen).tag(String?.none)
                            .help(context.nextTurnHasCurrent ? "不更改：沿用会话当前的推理强度" : "不指定：由运行时自己决定")
                        ForEach(context.reasoningEffortOptions, id: \.self) { level in Text(level).tag(Optional(level)) }
                    }.pickerStyle(.inline).labelsHidden()
                }
            }
        } label: {
            // Short by characters: a frame's limit does not hold where the menu takes its ideal width.
            Text(Self.pullDownTitle(compact ? Self.shortened(([model] + (effort.map { [$0] } ?? [])).joined(separator: " · "), to: 16) : title))
                .lineLimit(1).truncationMode(.middle).frame(maxWidth: 360, alignment: .leading)
        }
        .fixedSize()
        .accessibilityLabel("下一轮：运行时 \(runtime)，模型 \(model)，推理强度 \(effort ?? unchosen)")
        .help(context.chosenModelUnavailable ? "所选模型已不可用，下一轮改用\(model)" : "下一轮：\(runtime) · \(model) · 推理强度 \(effort ?? unchosen)")
        .disabled(store.isShuttingDown)
    }

    /// No choice first (the conversation's current model, or the runtime's
    /// default), then the runtime's own models, before the models Velune's
    /// providers serve through the gateway.
    @ViewBuilder private var modelMenuContent: some View {
        let context = self.context
        let choices = context.modelChoices
        let runtimeChoices = choices.filter { if case .managed = $0.selection { false } else { true } }
        let managedChoices = choices.filter { if case .managed = $0.selection { true } else { false } }
        let selection = Binding(get: { context.chosenModel }, set: { context.selectModel($0) })
        switch context.modelCatalog {
        case .loading: Text("正在读取运行时的模型…")
        case .failed(let detail): Text("无法读取运行时的模型：\(detail)")
        case .ready, nil: EmptyView()
        }
        if context.chosenModelUnavailable { Text("所选模型不可用，草稿已保留；请重新选择后发送。") }
        Picker("运行时模型", selection: selection) {
            Group {
                if context.nextTurnHasCurrent, context.nextTurnModel != nil { Text("沿用当前（\(context.selectedModelName ?? "")）") } else { Text("运行时默认") }
            }.tag(ModelSelection?.none)
                .help(context.nextTurnHasCurrent ? "不更改：沿用会话当前的模型" : "不指定：由运行时使用自己的默认模型")
            ForEach(runtimeChoices) { model in Text(model.name).tag(Optional(model.selection)) }
        }.pickerStyle(.inline).disabled(!context.canSwitchModel)
        if !managedChoices.isEmpty {
            Picker("Velune 提供商", selection: selection) {
                ForEach(managedChoices) { model in Text(model.name).tag(Optional(model.selection)) }
            }.pickerStyle(.inline).disabled(!context.canSwitchModel)
        }
    }

    @ViewBuilder private func sessionOptionControl(_ option: AcpConfigOption) -> some View {
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

    private func choiceItems(_ choices: [AcpConfigChoice]) -> some View {
        ForEach(choices, id: \.value) { choice in Text(choice.name).tag(choice.value).help(choice.description ?? choice.name) }
    }

    /// Choices in the Agent's order, consecutive ones of a group together.
    private static func choiceGroups(_ choices: [AcpConfigChoice]) -> [(offset: Int, name: String?, choices: [AcpConfigChoice])] {
        var groups: [(offset: Int, name: String?, choices: [AcpConfigChoice])] = []
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
    private func permissionMenu(compact: Bool) -> some View {
        let mode = context.effectivePermissionMode
        let current = context.currentPermissionMode
        let hasModes = !context.permissionModeOptions.isEmpty
        let title = hasModes ? (mode ?? current)?.label ?? "运行时默认权限" : "会话选项"
        return Menu {
            if hasModes {
                Picker("权限模式", selection: Binding(get: { mode?.id }, set: { context.selectPermissionMode($0) })) {
                    Text(current.map { "沿用当前（\($0.label)）" } ?? (context.nextTurnHasCurrent ? "沿用当前" : "运行时默认")).tag(String?.none)
                        .help(context.nextTurnHasCurrent ? "不更改：沿用会话当前的权限模式" : "不指定：由运行时使用自己的默认模式")
                    ForEach(context.permissionModeOptions) { option in Text(option.label).tag(Optional(option.id)).help(option.detail) }
                }.pickerStyle(.inline)
            }
            if !store.editableSessionOptions.isEmpty {
                Section("会话选项") {
                    ForEach(store.editableSessionOptions) { option in sessionOptionControl(option) }
                }.disabled(!store.canChangeSessionOptions)
            }
        } label: {
            if compact { Image(systemName: hasModes ? "lock.shield" : "slider.horizontal.3") }
            else { Label(Self.pullDownTitle(title), systemImage: hasModes ? "lock.shield" : "slider.horizontal.3").lineLimit(1) }
        }
        .fixedSize()
        .accessibilityLabel(hasModes ? "下一轮权限模式：\(title)" : "会话选项")
        .help(hasModes ? (mode.map { "权限模式：\($0.label)。\($0.detail)" } ?? (context.nextTurnHasCurrent ? "权限模式：沿用会话当前的模式\(current.map { "（\($0.label)）" } ?? "")" : "权限模式：由运行时使用自己的默认模式")) : "此会话由 Agent 提供的选项，修改立即生效")
        .disabled(store.isShuttingDown)
    }

    /// The system draws a borderless pull-down's chevron right against its
    /// title (and lays a custom label out as title and icon only), so the
    /// title carries a no-break space to keep the two apart.
    private static func pullDownTitle(_ title: String) -> String { title + "\u{00A0}" }
    private static func shortened(_ title: String, to length: Int) -> String {
        title.count <= length ? title : String(title.prefix(length / 2)) + "…" + String(title.suffix(length - length / 2 - 1))
    }

    /// Only a conversation that has not been sent yet can still pick where it works.
    private func workingDirectoryMenu(compact: Bool) -> some View {
        let path = store.loadedConversationWorkingDirectory
        let selectedProject = context.selectedWorkingProject
        let name = selectedProject?.name ?? path.map { URL(fileURLWithPath: $0).lastPathComponent }
        let detail = selectedProject.map { "项目：\($0.name)\n\($0.directories.joined(separator: "\n"))" } ?? path
        return Menu {
            if layout.canChooseLocalWorkingDirectory(context) {
                Button("选择目录…") { layout.chooseWorkingDirectory(context) }
            }
            Button("输入路径…") { layout.enterWorkingDirectory(context) }
            Button("无工作区") { context.selectWorkingDirectory(nil) }
            ForEach(context.workingDirectoryProjects, id: \.project) { project in
                Section(project.name) {
                    if project.canSelectProject {
                        Button("整个项目（\(project.directories.count) 个文件夹）") { context.selectWorkingProject(project) }
                    }
                    ForEach(project.directories, id: \.self) { directory in
                        Button(directory) { context.selectWorkingDirectory(directory) }
                    }
                }
            }
        } label: {
            Text(Self.pullDownTitle(compact ? Self.shortened(name ?? "无工作区", to: 8) : name ?? "无工作区")).lineLimit(1).truncationMode(.middle).frame(maxWidth: 160, alignment: .leading)
        }
        .fixedSize()
        .accessibilityLabel(detail ?? "无工作区")
        .help(detail ?? "无工作区：运行时允许时不指定目录；必须指定时使用执行机器的 ~/Documents/Velune")
        .disabled(!context.canChangeWorkingDirectory)
    }
}

/// An attached file above the composer: its thumbnail or icon, its name, and
/// a button to remove it.
private struct AttachmentChip: View {
    let url: URL
    let canRemove: Bool
    let retry: () -> Void
    let remove: () -> Void
    @State private var thumbnail: NSImage?
    @Environment(\.displayScale) private var displayScale

    var body: some View {
        HStack(spacing: 6) {
            Button {
                if !FileManager.default.isReadableFile(atPath: url.path) { retry() }
                else if !NSWorkspace.shared.open(url) { NSSound.beep() }
            } label: {
                HStack(spacing: 6) {
                    Image(nsImage: thumbnail ?? NSWorkspace.shared.icon(forFile: url.path))
                        .resizable().aspectRatio(contentMode: .fill)
                        .frame(width: 28, height: 28).clipShape(RoundedRectangle(cornerRadius: 4))
                    Text(url.lastPathComponent).lineLimit(1).truncationMode(.middle).frame(maxWidth: 160, alignment: .leading)
                }.contentShape(Rectangle())
            }
            .buttonStyle(.plain).accessibilityLabel("打开附件 \(url.lastPathComponent)")
            .help(FileManager.default.isReadableFile(atPath: url.path) ? "用默认应用打开 \(url.lastPathComponent)" : "附件不可用，点击重试同步")
            Button(action: remove) { Image(systemName: "xmark.circle.fill") }
                .buttonStyle(.borderless).foregroundStyle(.secondary)
                .disabled(!canRemove)
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

/// Recovery for input displaced by layout changes, away from the crowded
/// execution controls. Ordinary unsent text needs no separate draft action.
struct PreservedInputMenu: View {
    @ObservedObject var window: ConversationWindowModel
    let tab: ConversationTab
    @ObservedObject var model: ComposerModel
    var body: some View {
        Menu("保留的输入") {
            Button("另写一份输入") { window.newComposerDraft(for: tab) }
            let alternatives = window.composerDrafts.alternatives(to: model)
            if !alternatives.isEmpty {
                Divider()
                ForEach(alternatives, id: \.model.id) { item in
                    Button(item.name) { window.selectComposerDraft(item.model, for: tab) }
                }
            }
        }
    }
}
