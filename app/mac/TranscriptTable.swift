import SwiftUI
import AppKit

/// One explicit navigation request; the token distinguishes repeats of the same row.
struct TranscriptJump: Equatable {
    var id: String
    var token: UInt64
}

/// SwiftUI supplies the ordered presentation; the native table owns geometry,
/// row reuse, local disclosure state and the reading position.
struct TranscriptTable: NSViewRepresentable {
    let conversationID: String?
    let items: [TranscriptModel.Item]
    let revision: UInt64
    let activity: String?
    let outlineToggleIDs: Set<String>
    let outlineExpandedID: String?
    let cwd: String?
    let bottomRequest: UInt64
    let jump: TranscriptJump?
    let followingChanged: (Bool) -> Void
    let outlineToggle: (String) -> Void

    func makeCoordinator() -> TranscriptTableController { TranscriptTableController() }
    func makeNSView(context: Context) -> NSScrollView { context.coordinator.makeView() }
    func updateNSView(_ scroll: NSScrollView, context: Context) { context.coordinator.update(self) }
}

@MainActor final class TranscriptTableController: NSObject, NSTableViewDataSource, NSTableViewDelegate, NSTextViewDelegate {
    enum Entry {
        case message(TranscriptRow)
        case work(TranscriptTurn, label: String)
        case activity(String)
        case end
        var id: String {
            switch self {
            case .message(let row): return row.id
            case .work(let turn, _): return "work:" + turn.id
            case .activity: return "transcript-activity"
            case .end: return "transcript-end"
            }
        }
    }
    private enum ContentKey: Equatable {
        case message(UInt64, [String]), work(String, Bool), activity(String), end
    }
    private struct Measured {
        var key: ContentKey
        var width: CGFloat
        var layout: TranscriptRowLayout
    }
    /// Cancels an obsolete background pass without touching main-actor state.
    private final class MeasureToken: @unchecked Sendable {
        private let lock = NSLock()
        private var value = false
        var cancelled: Bool { lock.lock(); defer { lock.unlock() }; return value }
        func cancel() { lock.lock(); value = true; lock.unlock() }
    }
    private struct MeasureJob: @unchecked Sendable {
        var id: String
        var key: ContentKey
        var content: TranscriptRowContent
        var rendered: RenderedMessage?
    }
    private struct MeasureResult: @unchecked Sendable {
        var id: String
        var key: ContentKey
        var revision: UInt64?
        var rendered: RenderedMessage?
        var layout: TranscriptRowLayout
    }

    private var parent: TranscriptTable?
    private weak var scroll: NSScrollView?
    private weak var table: NSTableView?
    private var entries: [Entry] = []
    private var indexByID: [String: Int] = [:]
    private var conversationID: String?
    private var revision: UInt64?
    private var activity: String?
    private var outlineExpandedID: String?
    private var outlineToggleIDs: Set<String> = []
    private var expandedWork: Set<String> = []
    private var expandedBlocks: [String: Set<String>] = [:]

    private var rendered: [String: (revision: UInt64, value: RenderedMessage)] = [:]
    private var measured: [String: Measured] = [:]
    /// Height last handed to NSTableView, which caches it until told otherwise.
    private var committed: [String: CGFloat] = [:]
    private var lastKnown: [String: CGFloat] = [:]
    private let measurer = TextMeasurer()
    private let measureQueue = DispatchQueue(label: "local.velune.transcript-measure", qos: .utility)
    private var measureToken: MeasureToken?
    private var measureScheduled = false

    /// Provisional until the table reports its real column width.
    private var width: CGFloat = 800
    private var following = true
    private var programmatic = 0
    private var lastOrigin: CGFloat = 0
    private var lastBottomRequest: UInt64?
    private var lastJumpToken: UInt64?
    private var bandCheckScheduled = false

    // MARK: Setup

    func makeView() -> NSScrollView {
        let scroll = NSScrollView()
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.drawsBackground = true
        scroll.backgroundColor = .windowBackgroundColor
        let table = TranscriptTableView()
        table.headerView = nil
        table.backgroundColor = .windowBackgroundColor
        table.style = .fullWidth
        table.selectionHighlightStyle = .none
        table.intercellSpacing = .zero
        table.usesAutomaticRowHeights = false
        table.focusRingType = .none
        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("message"))
        column.resizingMask = .autoresizingMask
        table.addTableColumn(column)
        table.columnAutoresizingStyle = .noColumnAutoresizing
        table.dataSource = self
        table.delegate = self
        scroll.documentView = table
        self.scroll = scroll
        self.table = table
        scroll.contentView.postsBoundsChangedNotifications = true
        NotificationCenter.default.addObserver(self, selector: #selector(clipBoundsChanged), name: NSView.boundsDidChangeNotification, object: scroll.contentView)
        table.geometryChanged = { [weak self] in self?.tableFrameChanged() }
        scroll.contentView.postsFrameChangedNotifications = true
        NotificationCenter.default.addObserver(self, selector: #selector(clipFrameChanged), name: NSView.frameDidChangeNotification, object: scroll.contentView)
        NotificationCenter.default.addObserver(self, selector: #selector(liveResizeEnded), name: NSWindow.didEndLiveResizeNotification, object: nil)
        return scroll
    }

    // MARK: SwiftUI input

    func update(_ input: TranscriptTable) {
        parent = input
        guard let table else { return }
        if input.conversationID != conversationID {
            conversationID = input.conversationID
            expandedWork = []; expandedBlocks = [:]
            rendered = [:]; measured = [:]; committed = [:]; lastKnown = [:]
            entries = []; indexByID = [:]
            revision = nil
            following = true
            measureToken?.cancel()
            programmatically { table.reloadData() }
        }
        let structureChanged = input.revision != revision || input.activity != activity || input.outlineExpandedID != outlineExpandedID || input.outlineToggleIDs != outlineToggleIDs
        if structureChanged {
            revision = input.revision; activity = input.activity
            outlineExpandedID = input.outlineExpandedID; outlineToggleIDs = input.outlineToggleIDs
            rebuildEntries()
        }
        if lastBottomRequest != input.bottomRequest {
            lastBottomRequest = input.bottomRequest
            setFollowing(true)
            scrollToBottom()
        }
        if let jump = input.jump, jump.token != lastJumpToken {
            lastJumpToken = jump.token
            scrollTo(jump.id)
        }
    }

    private func rebuildEntries() {
        guard let table else { return }
        let liveTurns = Set((parent?.items ?? []).compactMap { item -> String? in
            if case .work(let turn, _) = item { return turn.id }
            return nil
        })
        expandedWork.formIntersection(liveTurns)
        var next: [Entry] = []
        for item in parent?.items ?? [] {
            switch item {
            case .message(let row): next.append(.message(row))
            case .work(let turn, let rows):
                next.append(.work(turn, label: Self.workLabel(turn)))
                if expandedWork.contains(turn.id) { next += rows.map(Entry.message) }
            }
        }
        if let activity = parent?.activity { next.append(.activity(activity)) }
        if !next.isEmpty { next.append(.end) }

        let oldIDs = entries.map(\.id)
        let newIDs = next.map(\.id)
        let anchor = captureAnchor()
        entries = next
        indexByID = Dictionary(newIDs.enumerated().map { ($1, $0) }, uniquingKeysWith: { first, _ in first })
        let live = Set(newIDs)
        rendered = rendered.filter { live.contains($0.key) }
        measured = measured.filter { live.contains($0.key) }
        committed = committed.filter { live.contains($0.key) }
        lastKnown = lastKnown.filter { live.contains($0.key) }
        expandedBlocks = expandedBlocks.filter { live.contains($0.key) }

        // Exact layouts for whatever will be on screen are computed before
        // the table sees the change, so no visible row starts at a guess.
        prepareVisibleBand(after: newIDs, anchor: anchor)

        programmatically {
            if oldIDs != newIDs {
                let difference = newIDs.difference(from: oldIDs)
                if difference.count > 200 || oldIDs.isEmpty {
                    table.reloadData()
                } else {
                    table.beginUpdates()
                    for change in difference.removals.reversed() { if case .remove(let offset, _, _) = change { table.removeRows(at: IndexSet(integer: offset), withAnimation: []) } }
                    for change in difference.insertions { if case .insert(let offset, _, _) = change { table.insertRows(at: IndexSet(integer: offset), withAnimation: []) } }
                    table.endUpdates()
                }
            }
            noteChangedHeights()
            reconfigureVisibleCells()
            restore(anchor)
        }
        scheduleBackgroundMeasure()
    }

    private func prepareVisibleBand(after ids: [String], anchor: (id: String, offset: CGFloat)?) {
        guard let scroll else { return }
        let viewport = scroll.contentView.bounds.height
        if following || anchor == nil {
            var filled: CGFloat = 0
            for index in entries.indices.reversed() {
                filled += exactLayout(index).height
                if filled > viewport * 1.5 { break }
            }
        } else if let anchor, let start = indexByID[anchor.id] {
            var filled: CGFloat = -anchor.offset
            for index in start ..< entries.count {
                filled += exactLayout(index).height
                if filled > viewport * 1.5 { break }
            }
        }
    }

    // MARK: Geometry

    private func content(_ index: Int) -> (ContentKey, TranscriptRowContent) {
        switch entries[index] {
        case .message(let row):
            let expanded = expandedBlocks[row.id] ?? []
            return (.message(row.contentRevision, expanded.sorted()), .message(row.message, revision: row.contentRevision, expanded: expanded))
        case .work(let turn, let label):
            let expanded = expandedWork.contains(turn.id)
            return (.work(label, expanded), .work(label: label, expanded: expanded))
        case .activity(let text):
            return (.activity(text), .activity(text))
        case .end:
            return (.end, .end)
        }
    }

    private func currentLayout(_ index: Int) -> TranscriptRowLayout? {
        let id = entries[index].id
        guard let value = measured[id], value.width == width, value.key == content(index).0 else { return nil }
        return value.layout
    }

    @discardableResult
    private func exactLayout(_ index: Int) -> TranscriptRowLayout {
        if let layout = currentLayout(index) { return layout }
        let (key, content) = content(index)
        let id = entries[index].id
        var renderedMessage: RenderedMessage?
        if case .message(let row) = entries[index] {
            if let cached = rendered[id], cached.revision == row.contentRevision { renderedMessage = cached.value }
            else {
                let value = RenderedMessage(row.message)
                rendered[id] = (row.contentRevision, value)
                renderedMessage = value
            }
        }
        let layout = TranscriptRowLayout.make(content, rendered: renderedMessage, width: width, measurer: measurer)
        measured[id] = Measured(key: key, width: width, layout: layout)
        lastKnown[id] = layout.height
        return layout
    }

    func numberOfRows(in tableView: NSTableView) -> Int { entries.count }

    func tableView(_ tableView: NSTableView, heightOfRow row: Int) -> CGFloat {
        guard entries.indices.contains(row) else { return 1 }
        let id = entries[row].id
        let height = currentLayout(row)?.height ?? lastKnown[id] ?? TranscriptRowLayout.estimate(content(row).1, width: width)
        committed[id] = height
        return max(height, 1)
    }

    /// Rows whose exact height differs from what the table believes.
    private func noteChangedHeights(_ candidates: [Int]? = nil) {
        guard let table else { return }
        var changed = IndexSet()
        for index in candidates ?? Array(entries.indices) {
            guard entries.indices.contains(index), let layout = currentLayout(index) else { continue }
            if committed[entries[index].id] != layout.height { changed.insert(index) }
        }
        guard !changed.isEmpty else { return }
        NSAnimationContext.beginGrouping()
        NSAnimationContext.current.duration = 0
        table.noteHeightOfRows(withIndexesChanged: changed)
        NSAnimationContext.endGrouping()
        table.tile()
    }

    private func captureAnchor() -> (id: String, offset: CGFloat)? {
        guard let scroll, let table, !entries.isEmpty else { return nil }
        let top = scroll.contentView.bounds.minY
        let row = table.row(at: NSPoint(x: 1, y: max(top, 0)))
        guard entries.indices.contains(row) else { return nil }
        return (entries[row].id, top - table.rect(ofRow: row).minY)
    }

    private func restore(_ anchor: (id: String, offset: CGFloat)?) {
        if following { scrollToBottom(); return }
        guard let anchor, let index = indexByID[anchor.id], let table else { return }
        setOrigin(table.rect(ofRow: index).minY + anchor.offset)
    }

    private func setOrigin(_ y: CGFloat) {
        guard let scroll else { return }
        let clip = scroll.contentView
        let desired = clip.constrainBoundsRect(NSRect(origin: NSPoint(x: clip.bounds.minX, y: y), size: clip.bounds.size)).origin
        guard abs(desired.y - clip.bounds.minY) > 0.25 else { return }
        programmatically {
            clip.setBoundsOrigin(desired)
            scroll.reflectScrolledClipView(clip)
        }
    }

    private func scrollToBottom() {
        guard let table else { return }
        setOrigin(table.bounds.maxY)
    }

    private func scrollTo(_ id: String) {
        guard let index = indexByID[id], let table, let scroll else { return }
        setFollowing(false)
        // Lay out the target and what follows it, then pin the target's top.
        var filled: CGFloat = 0
        for row in index ..< entries.count {
            filled += exactLayout(row).height
            if filled > scroll.contentView.bounds.height * 1.5 { break }
        }
        programmatically { noteChangedHeights() }
        setOrigin(table.rect(ofRow: index).minY + TranscriptRowLayout.rowInset - 8)
        reconfigureVisibleCells()
    }

    private func programmatically(_ body: () -> Void) {
        programmatic += 1
        body()
        programmatic -= 1
        if let scroll { lastOrigin = scroll.contentView.bounds.minY }
    }

    private var isAtBottom: Bool {
        guard let scroll, let table else { return true }
        let clip = scroll.contentView
        let bottom = clip.constrainBoundsRect(NSRect(origin: NSPoint(x: 0, y: table.bounds.maxY), size: clip.bounds.size)).minY
        return clip.bounds.minY >= bottom - 2
    }

    private func setFollowing(_ value: Bool) {
        guard following != value else { return }
        following = value
        parent?.followingChanged(value)
    }

    // MARK: Scrolling and resizing

    @objc private func clipBoundsChanged() {
        guard let scroll else { return }
        let origin = scroll.contentView.bounds.minY
        let userScrolled = programmatic == 0 && origin != lastOrigin
        lastOrigin = origin
        if userScrolled { setFollowing(isAtBottom) }
        // NSTableView defers row-height notes made inside its clip view's
        // bounds change, so geometry would land later without compensation.
        if programmatic == 0 { scheduleBandCheck() }
    }

    /// Rows about to appear get exact geometry synchronously; rows above the
    /// viewport that change height are compensated in the same pass.
    private func measureVisibleBand() {
        guard let table, let scroll, !entries.isEmpty else { return }
        for _ in 0 ..< 3 {
            let clip = scroll.contentView.bounds
            let range = table.rows(in: clip.insetBy(dx: 0, dy: -clip.height * 0.75))
            guard range.location != NSNotFound, range.length > 0 else { return }
            var pending: [Int] = []
            for index in range.location ..< min(range.location + range.length, entries.count) where currentLayout(index) == nil || committed[entries[index].id] != currentLayout(index)?.height {
                exactLayout(index)
                pending.append(index)
            }
            guard !pending.isEmpty else { return }
            let anchor = captureAnchor()
            programmatically {
                noteChangedHeights(pending)
                if following { scrollToBottom() } else { restore(anchor) }
                reconfigureVisibleCells()
            }
        }
    }

    @objc private func clipFrameChanged() { table?.tile() }

    private func tableFrameChanged() {
        guard let table else { return }
        let newWidth = table.tableColumns.first?.width ?? table.bounds.width
        guard newWidth > 0, newWidth != width else { return }
        width = newWidth
        measureToken?.cancel()
        // Off-screen rows keep their last known height until re-measured; the
        // visible band is re-measured outside this notification (see above).
        scheduleBandCheck()
        if !(table.window?.inLiveResize ?? false) { scheduleBackgroundMeasure() }
    }

    func tableViewColumnDidResize(_ notification: Notification) { tableFrameChanged() }

    @objc private func liveResizeEnded(_ notification: Notification) {
        guard (notification.object as? NSWindow) === table?.window else { return }
        scheduleBackgroundMeasure()
    }

    // MARK: Background measurement

    private func scheduleBackgroundMeasure() {
        guard !measureScheduled else { return }
        measureScheduled = true
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.measureScheduled = false
            self.startBackgroundMeasure()
        }
    }

    private func startBackgroundMeasure() {
        measureToken?.cancel()
        guard let table, width > 0, !entries.isEmpty else { return }
        let visible = table.rows(in: table.visibleRect)
        let center = visible.location == NSNotFound ? entries.count - 1 : visible.location + visible.length / 2
        // Nearest rows first: the reading position settles before distant history.
        let order = entries.indices.sorted { abs($0 - center) < abs($1 - center) }
        var jobs: [MeasureJob] = []
        for index in order where currentLayout(index) == nil {
            let (key, content) = content(index)
            var cached: RenderedMessage?
            if case .message(let row) = entries[index], let value = rendered[row.id], value.revision == row.contentRevision { cached = value.value }
            jobs.append(MeasureJob(id: entries[index].id, key: key, content: content, rendered: cached))
        }
        guard !jobs.isEmpty else { return }
        let token = MeasureToken()
        measureToken = token
        let width = self.width
        measureQueue.async { [weak self] in
            let measurer = TextMeasurer()
            var batch: [MeasureResult] = []
            var batchStart = DispatchTime.now()
            for job in jobs {
                if token.cancelled { return }
                var renderedMessage = job.rendered
                var revision: UInt64?
                if case .message(let message, let messageRevision, _) = job.content {
                    revision = messageRevision
                    if renderedMessage == nil { renderedMessage = RenderedMessage(message) }
                }
                let layout = TranscriptRowLayout.make(job.content, rendered: renderedMessage, width: width, measurer: measurer)
                batch.append(MeasureResult(id: job.id, key: job.key, revision: revision, rendered: renderedMessage, layout: layout))
                if batch.count >= 64 || DispatchTime.now().uptimeNanoseconds - batchStart.uptimeNanoseconds > 12_000_000 {
                    let results = batch
                    batch = []; batchStart = .now()
                    DispatchQueue.main.async { self?.apply(results, width: width, token: token) }
                }
            }
            let results = batch
            if !results.isEmpty { DispatchQueue.main.async { self?.apply(results, width: width, token: token) } }
        }
    }

    private func apply(_ results: [MeasureResult], width: CGFloat, token: MeasureToken) {
        guard !token.cancelled, width == self.width else { return }
        var changed: [Int] = []
        for result in results {
            guard let index = indexByID[result.id], content(index).0 == result.key else { continue }
            if let revision = result.revision, let value = result.rendered, rendered[result.id]?.revision != revision { rendered[result.id] = (revision, value) }
            measured[result.id] = Measured(key: result.key, width: width, layout: result.layout)
            lastKnown[result.id] = result.layout.height
            changed.append(index)
        }
        guard !changed.isEmpty else { return }
        let anchor = captureAnchor()
        programmatically {
            noteChangedHeights(changed)
            restore(anchor)
        }
    }

    // MARK: Cells

    func tableView(_ tableView: NSTableView, shouldSelectRow row: Int) -> Bool { false }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        guard entries.indices.contains(row) else { return nil }
        let identifier = NSUserInterfaceItemIdentifier("transcript-cell")
        let cell = tableView.makeView(withIdentifier: identifier, owner: nil) as? TranscriptCellView ?? TranscriptCellView()
        cell.identifier = identifier
        configure(cell, row: row)
        if currentLayout(row).map({ committed[entries[row].id] != $0.height }) ?? false { scheduleBandCheck() }
        return cell
    }

    private func scheduleBandCheck() {
        guard !bandCheckScheduled else { return }
        bandCheckScheduled = true
        DispatchQueue.main.async { [weak self] in
            self?.bandCheckScheduled = false
            self?.measureVisibleBand()
        }
    }

    private func configure(_ cell: TranscriptCellView, row: Int) {
        let entry = entries[row]
        let layout = exactLayout(row)
        var outlineToggle: (() -> Void)?
        var toggleHelp: String?
        if case .message(let row) = entry, outlineToggleIDs.contains(row.id) {
            outlineToggle = { [weak self] in self?.parent?.outlineToggle(row.id) }
            toggleHelp = outlineExpandedID == row.id ? "返回用户消息大纲" : "展开从此消息开始的会话"
        }
        cell.apply(layout, rowID: entry.id, outlineToggle: outlineToggle, toggleHelp: toggleHelp, textDelegate: self) { [weak self] key in
            self?.toggle(entryID: entry.id, key: key)
        }
    }

    private func reconfigureVisibleCells() {
        guard let table else { return }
        let range = table.rows(in: table.visibleRect)
        guard range.location != NSNotFound else { return }
        for index in range.location ..< min(range.location + range.length, entries.count) {
            if let cell = table.view(atColumn: 0, row: index, makeIfNecessary: false) as? TranscriptCellView { configure(cell, row: index) }
        }
    }

    private func toggle(entryID: String, key: String) {
        guard let table, let index = indexByID[entryID] else { return }
        let anchor = captureAnchor()
        if case .work(let turn, _) = entries[index] {
            if expandedWork.contains(turn.id) { expandedWork.remove(turn.id) } else { expandedWork.insert(turn.id) }
            let wasFollowing = following
            following = false
            rebuildEntries()
            following = wasFollowing
            programmatically { restore(anchor) }
        } else {
            var blocks = expandedBlocks[entryID] ?? []
            if blocks.contains(key) { blocks.remove(key) } else { blocks.insert(key) }
            expandedBlocks[entryID] = blocks
            exactLayout(index)
            // A local disclosure keeps the clicked row in place, even at the bottom.
            programmatically {
                noteChangedHeights([index])
                if let anchor, let anchorIndex = indexByID[anchor.id] { setOrigin(table.rect(ofRow: anchorIndex).minY + anchor.offset) }
                reconfigureVisibleCells()
            }
        }
        setFollowing(isAtBottom)
    }

    static func workLabel(_ turn: TranscriptTurn) -> String {
        if turn.isRunning { return "工作中" }
        guard let milliseconds = turn.durationMs else { return "工作过程" }
        let duration = Duration.milliseconds(Double(milliseconds)).formatted(.units(allowed: [.hours, .minutes, .seconds], width: .abbreviated, fractionalPart: .show(length: 1)))
        return "工作了 " + duration
    }

    // MARK: Context menu

    /// Adds a whole-message copy to the native text menu; a selection rarely
    /// spans every paragraph and code block of a long reply.
    func textView(_ view: NSTextView, menu: NSMenu, for event: NSEvent, at charIndex: Int) -> NSMenu? {
        guard let table, case let row = table.row(for: view), entries.indices.contains(row), case .message(let message) = entries[row] else { return menu }
        let source = message.message.blocks.compactMap { block -> String? in
            switch block {
            case .text(let text), .notice(let text): return text
            case .reasoning, .tool: return nil
            }
        }.joined(separator: "\n\n")
        guard !source.isEmpty else { return menu }
        let item = NSMenuItem(title: "拷贝整条消息", action: #selector(copyMessage(_:)), keyEquivalent: "")
        item.target = self
        item.representedObject = source
        menu.insertItem(item, at: 0)
        menu.insertItem(.separator(), at: 1)
        return menu
    }

    @objc private func copyMessage(_ sender: NSMenuItem) {
        guard let source = sender.representedObject as? String else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(source, forType: .string)
    }

    // MARK: Links

    func textView(_ textView: NSTextView, clickedOnLink link: Any, at charIndex: Int) -> Bool {
        let original: String
        if let url = link as? URL { original = url.absoluteString } else if let string = link as? String { original = string } else { return false }
        if let destination = resolve(original), NSWorkspace.shared.open(destination) { return true }
        let alert = NSAlert()
        alert.messageText = "无法打开链接"
        alert.informativeText = original
        alert.addButton(withTitle: "复制链接")
        alert.addButton(withTitle: "关闭")
        let copy = {
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(original, forType: .string)
        }
        if let window = textView.window {
            alert.beginSheetModal(for: window) { if $0 == .alertFirstButtonReturn { copy() } }
        } else if alert.runModal() == .alertFirstButtonReturn { copy() }
        return true
    }

    /// Relative file links belong to the conversation's working directory.
    private func resolve(_ original: String) -> URL? {
        if let url = URL(string: original), url.scheme != nil { return url }
        let path = original.removingPercentEncoding ?? original
        if path.hasPrefix("/") { return URL(fileURLWithPath: path) }
        guard let cwd = parent?.cwd, !path.isEmpty else { return nil }
        return URL(fileURLWithPath: cwd, isDirectory: true).appendingPathComponent(path).standardizedFileURL
    }
}

private final class TranscriptTableView: NSTableView {
    /// Column autoresizing happens while tiling, after frame notifications, so
    /// the controller reads the settled column width from here.
    var geometryChanged: (() -> Void)?
    override var acceptsFirstResponder: Bool { false }
    private var fitting = false
    override func tile() {
        // Keep the table exactly as wide as the viewport and let AppKit fit the
        // single column inside the style's padding; a wider column would widen
        // the table past the viewport.
        if !fitting, let clip = enclosingScrollView?.contentView, clip.bounds.width > 0 {
            fitting = true
            if frame.width != clip.bounds.width { setFrameSize(NSSize(width: clip.bounds.width, height: frame.height)) }
            sizeLastColumnToFit()
            fitting = false
        }
        super.tile()
        geometryChanged?()
    }
}

// MARK: - Cell views

/// Applies a precomputed layout; never measures or asks for its own size.
@MainActor final class TranscriptCellView: NSTableCellView {
    private var textViews: [TranscriptTextView] = []
    private var disclosures: [TranscriptDisclosureView] = []
    private var codeBlocks: [TranscriptCodeBlockView] = []
    private let bubble = TranscriptBubbleView()
    private var spinner: NSProgressIndicator?
    private var activityLabel: NSTextField?
    private var outlineToggle: (() -> Void)?

    override var isFlipped: Bool { true }

    override init(frame: NSRect) {
        super.init(frame: frame)
        addSubview(bubble)
        bubble.isHidden = true
        bubble.addGestureRecognizer(NSClickGestureRecognizer(target: self, action: #selector(bubbleClicked)))
    }
    convenience init() { self.init(frame: .zero) }
    required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }

    @objc private func bubbleClicked() { outlineToggle?() }

    func apply(_ layout: TranscriptRowLayout, rowID: String, outlineToggle: (() -> Void)?, toggleHelp: String?, textDelegate: NSTextViewDelegate, toggle: @escaping (String) -> Void) {
        self.outlineToggle = outlineToggle
        var textIndex = 0, disclosureIndex = 0, codeIndex = 0
        bubble.isHidden = true
        bubble.toolTip = toggleHelp
        spinner?.isHidden = true; spinner?.stopAnimation(nil)
        activityLabel?.isHidden = true
        for element in layout.elements {
            switch element {
            case .bubble(let frame):
                bubble.frame = frame
                bubble.isHidden = false
            case .text(let text, let frame, let inBubble):
                if textIndex == textViews.count {
                    let view = TranscriptTextView()
                    textViews.append(view)
                    addSubview(view)
                }
                let view = textViews[textIndex]; textIndex += 1
                view.delegate = textDelegate
                view.passesClicks = inBubble && outlineToggle != nil
                view.show(text, frame: frame)
            case .code(let language, let source, let text, let frame, let textFrame):
                if codeIndex == codeBlocks.count {
                    let view = TranscriptCodeBlockView()
                    codeBlocks.append(view)
                    addSubview(view)
                }
                let view = codeBlocks[codeIndex]; codeIndex += 1
                view.isHidden = false
                view.show(language: language, source: source, text: text, frame: frame,
                          textFrame: textFrame.offsetBy(dx: -frame.minX, dy: -frame.minY), textDelegate: textDelegate)
            case .disclosure(let key, let title, let symbol, let expanded, let frame):
                if disclosureIndex == disclosures.count {
                    let view = TranscriptDisclosureView()
                    disclosures.append(view)
                    addSubview(view)
                }
                let view = disclosures[disclosureIndex]; disclosureIndex += 1
                view.frame = frame
                view.configure(title: title, symbol: symbol, expanded: expanded) { toggle(key) }
                view.isHidden = false
            case .activity(let text, let frame):
                if spinner == nil {
                    let indicator = NSProgressIndicator()
                    indicator.style = .spinning; indicator.controlSize = .small; indicator.isDisplayedWhenStopped = false
                    let label = NSTextField(labelWithString: "")
                    label.font = TranscriptTypography.callout; label.textColor = .secondaryLabelColor
                    addSubview(indicator); addSubview(label)
                    spinner = indicator; activityLabel = label
                }
                spinner?.frame = NSRect(x: frame.minX, y: frame.minY + 1, width: 16, height: 16)
                spinner?.isHidden = false; spinner?.startAnimation(nil)
                activityLabel?.stringValue = text
                activityLabel?.frame = NSRect(x: frame.minX + 22, y: frame.minY, width: frame.width - 22, height: frame.height)
                activityLabel?.isHidden = false
            }
        }
        for view in textViews[textIndex...] { view.isHidden = true; view.clear() }
        for view in disclosures[disclosureIndex...] { view.isHidden = true }
        for view in codeBlocks[codeIndex...] { view.isHidden = true; view.clear() }
    }
}

/// TextKit 1 text view configured exactly like `TextMeasurer`.
@MainActor final class TranscriptTextView: NSTextView {
    var passesClicks = false

    init() {
        let storage = NSTextStorage()
        let layoutManager = NSLayoutManager()
        storage.addLayoutManager(layoutManager)
        let container = NSTextContainer(size: NSSize(width: 100, height: CGFloat.greatestFiniteMagnitude))
        container.lineFragmentPadding = 0
        container.widthTracksTextView = false
        container.heightTracksTextView = false
        layoutManager.addTextContainer(container)
        super.init(frame: .zero, textContainer: container)
        isEditable = false
        isSelectable = true
        drawsBackground = false
        textContainerInset = .zero
        isVerticallyResizable = false
        isHorizontallyResizable = false
        isAutomaticLinkDetectionEnabled = false
        usesFindBar = false
        // Become layer-backed while the frame is still empty: AppKit pixel-aligns
        // the frame when backing is created, which would alter measured geometry.
        wantsLayer = true
    }
    override init(frame frameRect: NSRect, textContainer container: NSTextContainer?) { super.init(frame: frameRect, textContainer: container) }
    required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }

    private var shown: NSAttributedString?

    func show(_ text: NSAttributedString, frame: NSRect) {
        isHidden = false
        if self.frame != frame {
            self.frame = frame
            textContainer?.size = NSSize(width: frame.width, height: CGFloat.greatestFiniteMagnitude)
        }
        guard shown !== text else { return }
        shown = text
        setSelectedRange(NSRange(location: 0, length: 0))
        textStorage?.setAttributedString(text)
    }

    func clear() {
        guard shown != nil else { return }
        shown = nil
        textStorage?.setAttributedString(NSAttributedString())
    }

    override func hitTest(_ point: NSPoint) -> NSView? { passesClicks ? nil : super.hitTest(point) }
}

@MainActor private final class TranscriptBubbleView: NSView {
    override func draw(_ dirtyRect: NSRect) {
        NSColor.quaternaryLabelColor.setFill()
        NSBezierPath(roundedRect: bounds, xRadius: 12, yRadius: 12).fill()
    }
}

/// A message fold header: native disclosure triangle plus its clickable label.
@MainActor private final class TranscriptDisclosureView: NSView {
    private let triangle = NSButton()
    private let label = NSButton()
    private var action: (() -> Void)?
    override var isFlipped: Bool { true }

    override init(frame: NSRect) {
        super.init(frame: frame)
        triangle.bezelStyle = .disclosure
        triangle.setButtonType(.pushOnPushOff)
        triangle.title = ""
        triangle.controlSize = .small
        triangle.target = self; triangle.action = #selector(clicked)
        triangle.sizeToFit()
        label.isBordered = false
        label.imagePosition = .imageLeading
        label.alignment = .left
        label.contentTintColor = .secondaryLabelColor
        label.target = self; label.action = #selector(clicked)
        addSubview(triangle); addSubview(label)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }

    func configure(title: String, symbol: String?, expanded: Bool, action: @escaping () -> Void) {
        self.action = action
        triangle.state = expanded ? .on : .off
        triangle.setAccessibilityLabel(title)
        label.attributedTitle = NSAttributedString(string: title, attributes: [.font: TranscriptTypography.callout, .foregroundColor: NSColor.secondaryLabelColor])
        label.image = symbol.flatMap { NSImage(systemSymbolName: $0, accessibilityDescription: nil)?.withSymbolConfiguration(.init(pointSize: 11, weight: .regular)) }
        needsLayout = true
    }

    override func layout() {
        super.layout()
        let size = triangle.fittingSize
        triangle.frame = NSRect(x: 0, y: (bounds.height - size.height) / 2, width: size.width, height: size.height)
        let labelWidth = min(label.fittingSize.width, bounds.width - size.width - 6)
        label.frame = NSRect(x: size.width + 6, y: 0, width: max(labelWidth, 0), height: bounds.height)
    }

    @objc private func clicked(_ sender: NSButton) {
        // The layout owns expansion; reconfiguration sets the triangle state.
        action?()
    }
}

/// A fenced code block: rounded background, language label, copy control and
/// selectable monospaced text, positioned from the precomputed row layout.
@MainActor private final class TranscriptCodeBlockView: NSView {
    private let label = NSTextField(labelWithString: "")
    private let copyButton = CodeCopyButton()
    private let text = TranscriptTextView()
    override var isFlipped: Bool { true }

    override init(frame: NSRect) {
        super.init(frame: frame)
        label.font = TranscriptTypography.caption
        label.textColor = .secondaryLabelColor
        addSubview(label); addSubview(copyButton); addSubview(text)
    }
    convenience init() { self.init(frame: .zero) }
    required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }

    func show(language: String?, source: String, text value: NSAttributedString, frame: NSRect, textFrame: NSRect, textDelegate: NSTextViewDelegate) {
        self.frame = frame
        label.stringValue = language ?? "代码"
        let padding = TranscriptRowLayout.codePadding, labelHeight = TranscriptRowLayout.codeLabelHeight
        label.sizeToFit()
        label.frame.origin = NSPoint(x: padding, y: padding + (labelHeight - label.frame.height) / 2)
        copyButton.code = source
        copyButton.sizeToFit()
        copyButton.frame.origin = NSPoint(x: frame.width - padding - copyButton.frame.width, y: padding + (labelHeight - copyButton.frame.height) / 2)
        text.delegate = textDelegate
        text.show(value, frame: textFrame)
    }

    func clear() { text.clear() }

    override func draw(_ dirtyRect: NSRect) {
        NSColor.quaternarySystemFill.setFill()
        NSBezierPath(roundedRect: bounds, xRadius: 6, yRadius: 6).fill()
    }
}

@MainActor private final class CodeCopyButton: NSButton {
    var code = "" { didSet { if oldValue != code { reset() } } }
    private var resetWork: DispatchWorkItem?

    init() {
        super.init(frame: .zero)
        bezelStyle = .accessoryBarAction
        controlSize = .small
        font = TranscriptTypography.caption
        target = self; action = #selector(copyCode)
        reset()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }

    private func reset() {
        resetWork?.cancel()
        title = "复制"
        image = NSImage(systemSymbolName: "doc.on.doc", accessibilityDescription: nil)
        imagePosition = .imageLeading
        toolTip = "复制代码"
    }

    @objc private func copyCode() {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(code, forType: .string)
        title = "已复制"
        image = NSImage(systemSymbolName: "checkmark", accessibilityDescription: nil)
        let work = DispatchWorkItem { [weak self] in self?.reset() }
        resetWork = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 2, execute: work)
    }
}
