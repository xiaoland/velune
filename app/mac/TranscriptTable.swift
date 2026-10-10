import SwiftUI
import AppKit

/// One explicit navigation request; the token distinguishes repeats of the same row.
struct TranscriptJump: Equatable {
    var id: String
    var token: UInt64
}

/// SwiftUI supplies the ordered presentation; the native list owns geometry,
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
    /// Read deferred content of messages with an expanded disclosure.
    let loadPayloads: ([String]) -> Void

    func makeCoordinator() -> TranscriptTableController { TranscriptTableController() }
    func makeNSView(context: Context) -> NSScrollView { context.coordinator.makeView() }
    func updateNSView(_ scroll: NSScrollView, context: Context) { context.coordinator.update(self) }
}

/// A virtualized transcript. Row geometry is exactly the controller's height
/// list (prefix sums), so a row moves only when the controller changes a height,
/// and every such change restores the reading anchor in the same main-thread
/// step. NSTableView is deliberately not used: it estimates unmeasured rows,
/// measures them lazily from arbitrary calls and re-anchors on its own.
@MainActor final class TranscriptTableController: NSObject, NSTextViewDelegate {
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
    private typealias Anchor = (id: String, offset: CGFloat)

    private var parent: TranscriptTable?
    private weak var scroll: NSScrollView?
    private weak var document: TranscriptDocumentView?
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
    /// Displayed geometry: one height per entry and their prefix offsets.
    private var heights: [CGFloat] = []
    private var offsets: [CGFloat] = [0]
    private var lastKnown: [String: CGFloat] = [:]
    private let measurer = TextMeasurer()
    private let measureQueue = DispatchQueue(label: "local.velune.transcript-measure", qos: .utility)
    private var measureToken: MeasureToken?
    private var measureScheduled = false

    private var visibleCells: [String: TranscriptCellView] = [:]
    private var reusableCells: [TranscriptCellView] = []

    /// Provisional until the clip view reports its real width.
    private var width: CGFloat = 800
    private var following = true
    private var programmatic = 0
    private var lastOrigin: CGFloat = 0
    private var lastBottomRequest: UInt64?
    private var lastJumpToken: UInt64?

    // MARK: Setup

    func makeView() -> NSScrollView {
        let scroll = NSScrollView()
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.drawsBackground = true
        scroll.backgroundColor = .windowBackgroundColor
        let document = TranscriptDocumentView()
        document.controller = self
        document.frame = NSRect(x: 0, y: 0, width: width, height: 0)
        scroll.documentView = document
        self.scroll = scroll
        self.document = document
        let clip = scroll.contentView
        clip.postsBoundsChangedNotifications = true
        clip.postsFrameChangedNotifications = true
        NotificationCenter.default.addObserver(self, selector: #selector(clipBoundsChanged), name: NSView.boundsDidChangeNotification, object: clip)
        NotificationCenter.default.addObserver(self, selector: #selector(clipFrameChanged), name: NSView.frameDidChangeNotification, object: clip)
        NotificationCenter.default.addObserver(self, selector: #selector(liveResizeEnded), name: NSWindow.didEndLiveResizeNotification, object: nil)
        return scroll
    }

    // MARK: SwiftUI input

    func update(_ input: TranscriptTable) {
        parent = input
        clipFrameChanged()
        if input.conversationID != conversationID {
            conversationID = input.conversationID
            expandedWork = []; expandedBlocks = [:]
            rendered = [:]; measured = [:]; lastKnown = [:]
            entries = []; indexByID = [:]; heights = []; offsets = [0]
            for cell in visibleCells.values { recycle(cell) }
            visibleCells = [:]
            revision = nil
            following = true
            measureToken?.cancel()
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
            materializeVisibleRows()
        }
        if let jump = input.jump, jump.token != lastJumpToken {
            lastJumpToken = jump.token
            scrollTo(jump.id)
        }
    }

    private func rebuildEntries() {
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

        let anchor = captureAnchor()
        let previousHeights = Dictionary(entries.indices.map { (entries[$0].id, heights[$0]) }, uniquingKeysWith: { first, _ in first })
        entries = next
        indexByID = Dictionary(next.indices.map { (next[$0].id, $0) }, uniquingKeysWith: { first, _ in first })
        let live = Set(indexByID.keys)
        rendered = rendered.filter { live.contains($0.key) }
        measured = measured.filter { live.contains($0.key) }
        lastKnown = lastKnown.filter { live.contains($0.key) }
        expandedBlocks = expandedBlocks.filter { live.contains($0.key) }
        for (id, cell) in visibleCells where !live.contains(id) { recycle(cell); visibleCells[id] = nil }

        // Rows keep their displayed height; changes go through the exact
        // layouts below, measured before anything is drawn.
        heights = entries.indices.map { index in previousHeights[entries[index].id] ?? fallbackHeight(index) }
        recomputeOffsets(from: 0)
        prepareBand(around: anchor)
        commitExactHeights(in: entries.indices, keeping: anchor)
        scheduleBackgroundMeasure()
        requestExpandedPayloads()
    }

    /// Expanded disclosures show a placeholder until their content is read;
    /// a newer delivery of the same message is read again while it stays expanded.
    private func requestExpandedPayloads() {
        let ids = expandedBlocks.compactMap { rowID, keys -> String? in
            guard let index = indexByID[rowID], case .message(let row) = entries[index],
                  !keys.isDisjoint(with: RenderedMessage.deferredDisclosureKeys(row.message)) else { return nil }
            return row.message.id
        }
        if !ids.isEmpty { parent?.loadPayloads(ids) }
    }

    /// Exact layouts for the rows that will be on screen after a change.
    private func prepareBand(around anchor: Anchor?) {
        guard let scroll else { return }
        let viewport = scroll.contentView.bounds.height
        if following || anchor == nil {
            var filled: CGFloat = 0
            for index in entries.indices.reversed() {
                filled += exactLayout(index).height
                if filled > viewport * 1.5 { break }
            }
        } else if let anchor, let start = indexByID[anchor.id] {
            var filled = -anchor.offset
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
        guard let value = measured[entries[index].id], value.width == width, value.key == content(index).0 else { return nil }
        return value.layout
    }

    private func fallbackHeight(_ index: Int) -> CGFloat {
        currentLayout(index)?.height ?? lastKnown[entries[index].id] ?? TranscriptRowLayout.estimate(content(index).1, width: width)
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

    private func recomputeOffsets(from start: Int) {
        if offsets.count != heights.count + 1 { offsets = Array(repeating: 0, count: heights.count + 1) }
        var y = start == 0 ? 0 : offsets[start]
        for index in start ..< heights.count {
            offsets[index] = y
            y += heights[index]
        }
        offsets[heights.count] = y
        if let document, let scroll {
            let size = NSSize(width: scroll.contentView.bounds.width, height: y)
            if document.frame.size != size { programmatically { document.setFrameSize(size) } }
        }
    }

    private func rowRect(_ index: Int) -> NSRect {
        NSRect(x: 0, y: offsets[index], width: document?.bounds.width ?? width, height: heights[index])
    }

    /// The row containing `y`, clamped to the list.
    private func row(at y: CGFloat) -> Int? {
        guard !heights.isEmpty else { return nil }
        var low = 0, high = heights.count - 1
        while low < high {
            let middle = (low + high + 1) / 2
            if offsets[middle] <= y { low = middle } else { high = middle - 1 }
        }
        return low
    }

    private func rows(in rect: NSRect) -> Range<Int> {
        guard let first = row(at: max(rect.minY, 0)), let last = row(at: rect.maxY) else { return 0 ..< 0 }
        return first ..< last + 1
    }

    /// Applies exact heights for `indices` and restores the anchor (or the
    /// bottom) in the same step; the only place displayed geometry changes.
    private func commitExactHeights(in indices: some Sequence<Int>, keeping anchor: Anchor?) {
        applyExactHeights(in: indices, keeping: anchor)
        materializeVisibleRows()
    }

    private func applyExactHeights(in indices: some Sequence<Int>, keeping anchor: Anchor?) {
        var first: Int?
        for index in indices {
            guard let layout = currentLayout(index), heights[index] != layout.height else { continue }
            heights[index] = layout.height
            first = min(first ?? index, index)
        }
        if let first { recomputeOffsets(from: first) }
        restore(anchor)
    }

    private func captureAnchor() -> Anchor? {
        guard let scroll, let index = row(at: scroll.contentView.bounds.minY), entries.indices.contains(index) else { return nil }
        return (entries[index].id, scroll.contentView.bounds.minY - offsets[index])
    }

    private func restore(_ anchor: Anchor?) {
        if following { scrollToBottom(); return }
        guard let anchor, let index = indexByID[anchor.id] else { return }
        setOrigin(offsets[index] + anchor.offset)
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

    private func scrollToBottom() { setOrigin(offsets.last ?? 0) }

    private func scrollTo(_ id: String) {
        guard let index = indexByID[id], let scroll else { return }
        setFollowing(false)
        // Lay out the target and what follows it, then pin the target's top.
        var filled: CGFloat = 0
        var last = index
        for row in index ..< entries.count {
            filled += exactLayout(row).height
            last = row
            if filled > scroll.contentView.bounds.height * 1.5 { break }
        }
        commitExactHeights(in: index ... last, keeping: nil)
        setOrigin(offsets[index] + TranscriptRowLayout.rowInset - 8)
        materializeVisibleRows()
    }

    private func programmatically(_ body: () -> Void) {
        programmatic += 1
        body()
        programmatic -= 1
        if let scroll { lastOrigin = scroll.contentView.bounds.minY }
    }

    private var isAtBottom: Bool {
        guard let scroll else { return true }
        let clip = scroll.contentView
        let bottom = clip.constrainBoundsRect(NSRect(origin: NSPoint(x: 0, y: offsets.last ?? 0), size: clip.bounds.size)).minY
        return clip.bounds.minY >= bottom - 2
    }

    private func setFollowing(_ value: Bool) {
        guard following != value else { return }
        following = value
        parent?.followingChanged(value)
    }

    // MARK: Scrolling and resizing

    @objc private func clipBoundsChanged() {
        guard programmatic == 0, let scroll else { return }
        let origin = scroll.contentView.bounds.minY
        if origin != lastOrigin {
            lastOrigin = origin
            setFollowing(isAtBottom)
        }
        measureVisibleBand()
    }

    /// Rows about to appear get exact geometry synchronously, before drawing;
    /// rows above the viewport that change height are compensated in place.
    private func measureVisibleBand() {
        guard let scroll, !entries.isEmpty else { return }
        for _ in 0 ..< 3 {
            let clip = scroll.contentView.bounds
            let band = rows(in: clip.insetBy(dx: 0, dy: -clip.height * 0.75))
            var pending: [Int] = []
            for index in band where heights[index] != exactLayout(index).height { pending.append(index) }
            guard !pending.isEmpty else { break }
            commitExactHeights(in: pending, keeping: captureAnchor())
        }
        materializeVisibleRows()
    }

    @objc private func clipFrameChanged() {
        guard let scroll, let document else { return }
        let clipWidth = scroll.contentView.bounds.width
        guard clipWidth > 0 else { return }
        if document.frame.width != clipWidth { programmatically { document.setFrameSize(NSSize(width: clipWidth, height: offsets.last ?? 0)) } }
        guard clipWidth != width else { materializeVisibleRows(); return }
        let anchor = captureAnchor()
        width = clipWidth
        measureToken?.cancel()
        // Off-screen rows keep their last displayed height until re-measured.
        prepareBand(around: anchor)
        commitExactHeights(in: entries.indices, keeping: anchor)
        if !(scroll.window?.inLiveResize ?? false) { scheduleBackgroundMeasure() }
    }

    @objc private func liveResizeEnded(_ notification: Notification) {
        guard (notification.object as? NSWindow) === scroll?.window else { return }
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
        guard let scroll, !entries.isEmpty else { return }
        let center = row(at: scroll.contentView.bounds.midY) ?? entries.count - 1
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
        commitExactHeights(in: changed, keeping: captureAnchor())
    }

    // MARK: Cells

    /// Materializes and positions cells for the viewport plus overdraw; cells
    /// leaving it are kept for reuse.
    func materializeVisibleRows(extra rect: NSRect? = nil) {
        guard let scroll, let document else { return }
        func area() -> NSRect { scroll.contentView.bounds.insetBy(dx: 0, dy: -scroll.contentView.bounds.height * 0.5) }
        // A cell draws its row's exact layout, so a row is shown only at that
        // layout's committed height; otherwise its content covers the rows
        // below. Rows near the viewport that no earlier step measured (after a
        // width or content change) are committed here, keeping the reading
        // anchor; that can bring further rows into the area until it settles.
        while true {
            let pending = rows(in: area()).filter { heights[$0] != exactLayout($0).height }
            guard !pending.isEmpty else { break }
            applyExactHeights(in: pending, keeping: captureAnchor())
        }
        var range = Array(rows(in: area()))
        // AppKit's responsive-scrolling overdraw only prepares rows that are
        // already consistent; geometry never changes while it prepares content.
        if let rect {
            let core = Set(range)
            range += rows(in: rect).filter { !core.contains($0) && currentLayout($0)?.height == heights[$0] }
        }
        let wanted = Set(range.map { entries[$0].id })
        for (id, cell) in visibleCells where !wanted.contains(id) { recycle(cell); visibleCells[id] = nil }
        for index in range {
            let id = entries[index].id
            let cell: TranscriptCellView
            if let existing = visibleCells[id] { cell = existing }
            else {
                cell = reusableCells.popLast() ?? TranscriptCellView()
                if cell.superview !== document { document.addSubview(cell) }
                cell.isHidden = false
                visibleCells[id] = cell
            }
            configure(cell, row: index)
            let frame = rowRect(index)
            if cell.frame != frame { cell.frame = frame }
        }
    }

    private func recycle(_ cell: TranscriptCellView) {
        // NSView.prepareForReuse unhides the view; hide afterwards so a parked
        // cell never shows its stale headers over the rows now in its place.
        cell.prepareForReuse()
        cell.isHidden = true
        reusableCells.append(cell)
    }

    private func configure(_ cell: TranscriptCellView, row: Int) {
        let entry = entries[row]
        let layout = exactLayout(row)
        let toggles = outlineToggleIDs.contains(entry.id)
        let token = "\(content(row).0)|\(width)|\(toggles)|\(outlineExpandedID == entry.id)"
        guard cell.configurationToken != token || cell.rowID != entry.id else { return }
        var outlineToggle: (() -> Void)?
        var toggleHelp: String?
        if toggles {
            outlineToggle = { [weak self] in self?.parent?.outlineToggle(entry.id) }
            toggleHelp = outlineExpandedID == entry.id ? "返回用户消息大纲" : "展开从此消息开始的会话"
        }
        cell.apply(layout, rowID: entry.id, outlineToggle: outlineToggle, toggleHelp: toggleHelp, textDelegate: self) { [weak self] key in
            self?.toggle(entryID: entry.id, key: key)
        }
        cell.configurationToken = token
    }

    private func toggle(entryID: String, key: String) {
        guard let index = indexByID[entryID] else { return }
        // A local disclosure keeps the reading position, even at the bottom.
        let anchor = captureAnchor()
        let wasFollowing = following
        following = false
        if case .work(let turn, _) = entries[index] {
            if expandedWork.contains(turn.id) { expandedWork.remove(turn.id) } else { expandedWork.insert(turn.id) }
            rebuildEntries()
            restore(anchor)
        } else {
            var blocks = expandedBlocks[entryID] ?? []
            if blocks.contains(key) { blocks.remove(key) } else { blocks.insert(key) }
            expandedBlocks[entryID] = blocks
            exactLayout(index)
            commitExactHeights(in: [index], keeping: anchor)
            requestExpandedPayloads()
        }
        following = wasFollowing
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
        var ancestor: NSView? = view
        while let current = ancestor, !(current is TranscriptCellView) { ancestor = current.superview }
        guard let rowID = (ancestor as? TranscriptCellView)?.rowID, let row = indexByID[rowID], case .message(let message) = entries[row] else { return menu }
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

/// Flipped document whose height is the controller's total row height.
@MainActor final class TranscriptDocumentView: NSView {
    weak var controller: TranscriptTableController?
    override var isFlipped: Bool { true }
    override var acceptsFirstResponder: Bool { false }
    /// Responsive scrolling asks for overdraw before it is visible.
    override func prepareContent(in rect: NSRect) {
        super.prepareContent(in: rect)
        controller?.materializeVisibleRows(extra: rect)
    }
}

// MARK: - Cell views

/// Applies a precomputed layout; never measures or asks for its own size.
@MainActor final class TranscriptCellView: NSView {
    private(set) var rowID: String?
    /// Identifies the applied layout so scrolling does not reapply it.
    var configurationToken: String?
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

    /// Drops text so hidden reusable cells keep no stale selection or storage.
    override func prepareForReuse() {
        super.prepareForReuse()
        rowID = nil
        configurationToken = nil
        outlineToggle = nil
        for view in textViews { view.clear() }
        for view in codeBlocks { view.clear() }
        spinner?.stopAnimation(nil)
    }

    func apply(_ layout: TranscriptRowLayout, rowID: String, outlineToggle: (() -> Void)?, toggleHelp: String?, textDelegate: NSTextViewDelegate, toggle: @escaping (String) -> Void) {
        self.rowID = rowID
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
