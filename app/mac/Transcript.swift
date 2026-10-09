import SwiftUI
import AppKit
import VeluneBindings

struct TranscriptView: View {
    @ObservedObject var model: TranscriptModel
    let conversationID: String?
    let scrollRequest: UInt64
    let sentAfterUserID: String?
    let cwd: String?
    @Binding var showsOutline: Bool
    let activity: String?
    let presentation: BindingTranscriptPresentation
    @State private var following = true
    @State private var outline = TranscriptOutlineState()
    @State private var jumpTarget: String?
    @State private var expandedWork: Set<String> = []
    private var visibleRows: [TranscriptRow] { presentation == .userOutline ? outline.visibleRows(model.rows, userIDs: Set(model.userRows.map(\.id))) : model.rows }
    @State private var bottomRequest: UInt64 = 0
    var body: some View {
        let items = model.items(for: visibleRows).flatMap { item -> [TranscriptModel.Item] in
            if case .work(let turn, let rows) = item, expandedWork.contains(turn.id) {
                return [item] + rows.map(TranscriptModel.Item.message)
            }
            return [item]
        }
        let ids = items.map(\.scrollID) + (activity == nil ? [] : ["transcript-activity"])
        var contentRevisions = Dictionary(uniqueKeysWithValues: items.map { item -> (String, NativeTranscriptContentRevision) in
            switch item {
            case .message(let row): return (row.id, .message(row.contentRevision))
            case .work(let turn, _): return (item.scrollID, .work(workLabel(turn), expandedWork.contains(turn.id)))
            }
        })
        if let activity { contentRevisions["transcript-activity"] = .activity(activity) }
        return NativeTranscriptTable(ids: ids, contentRevisions: contentRevisions, revision: model.contentRevision,
            layoutKey: expandedWork.sorted().joined(separator: "|") + String(describing: presentation) + (activity ?? ""),
            followsBottom: following, jumpID: jumpTarget, bottomRequest: bottomRequest,
            content: { id in
                if id == "transcript-activity", let activity { return AnyView(HStack { ProgressView().controlSize(.small); Text(activity).foregroundStyle(.secondary) }.font(.callout)) }
                guard let item = items.first(where: { $0.scrollID == id }) else { return AnyView(EmptyView()) }
                return AnyView(itemView(item))
            }, scrollIntent: { following = false; jumpTarget = nil },
            jumpCompleted: { id in if jumpTarget == id { jumpTarget = nil } })
            .background(Color(nsColor: .windowBackgroundColor))
            .overlay(alignment: .bottomTrailing) {
                if !following {
                    Button { following = true; jumpTarget = nil; bottomRequest &+= 1 } label: { Image(systemName: "arrow.down") }
                        .buttonStyle(.bordered).help("回到底部").accessibilityLabel("回到底部").padding(16)
                }
            }
            .sheet(isPresented: $showsOutline) {
                TranscriptOutlineView(model: model) { id in
                    following = false
                    if presentation == .userOutline { _ = outline.expand(from: id, in: model.rows) }
                    jumpTarget = id
                }
            }
            .onChange(of: conversationID) { _, _ in outline = TranscriptOutlineState(); expandedWork = []; following = true; jumpTarget = nil; bottomRequest &+= 1 }
            .onChange(of: presentation) { _, _ in outline = TranscriptOutlineState(); following = true; jumpTarget = nil; bottomRequest &+= 1 }
            .onChange(of: scrollRequest) { _, _ in
                if presentation == .userOutline { outline.followSentUser(after: sentAfterUserID, in: model.rows) }
                following = true; jumpTarget = nil; bottomRequest &+= 1
            }
            .onChange(of: model.contentRevision) { _, _ in
                expandedWork = model.retainedExpandedTurnIDs(expandedWork)
                if outline.reconcile(model.rows) { following = false }
            }
    }
    @ViewBuilder private func itemView(_ item: TranscriptModel.Item) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            switch item {
            case .message(let row):
                if presentation == .userOutline && row.message.role == .user {
                    Button {
                        following = false
                        if outline.expandedFromUserID == row.id { outline.collapse() }
                        else { _ = outline.expand(from: row.id, in: model.rows) }
                        jumpTarget = row.id
                    } label: { ConversationMessageView(row: row, cwd: cwd) }
                        .buttonStyle(.plain).help(outline.expandedFromUserID == row.id ? "返回用户消息大纲" : "展开从此消息开始的会话")
                } else { ConversationMessageView(row: row, cwd: cwd) }
            case .work(let turn, _):
                InlineMessageDisclosure(isExpanded: workExpansion(turn.id), accessibilityLabel: workLabel(turn)) {
                    EmptyView()
                } label: { Text(workLabel(turn)).font(.callout).foregroundStyle(.secondary) }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }.frame(maxWidth: 760, alignment: .leading).frame(maxWidth: .infinity)
    }
    private func workExpansion(_ id: String) -> Binding<Bool> {
        Binding(get: { expandedWork.contains(id) }, set: { if $0 { expandedWork.insert(id) } else { expandedWork.remove(id) } })
    }
    private func workLabel(_ turn: TranscriptTurn) -> String {
        if turn.isRunning { return "工作中" }
        guard let milliseconds = turn.durationMs else { return "工作过程" }
        let duration = Duration.milliseconds(Double(milliseconds)).formatted(.units(allowed: [.hours, .minutes, .seconds], width: .abbreviated, fractionalPart: .show(length: 1)))
        return "工作了 " + duration
    }
}

@MainActor private extension TranscriptModel.Item {
    var scrollID: String {
        switch self {
        case .message(let row): return row.id
        case .work(let turn, _): return "work:" + turn.id
        }
    }
}

/// The native table owns row materialization and exact row-index navigation.
/// SwiftUI only supplies the content of reusable visible cells.
private enum NativeTranscriptContentRevision: Equatable {
    case message(UInt64), work(String, Bool), activity(String)
}
private struct NativeTranscriptTable: NSViewRepresentable {
    private static let contentInset: CGFloat = 24
    let ids: [String]
    let contentRevisions: [String: NativeTranscriptContentRevision]
    let revision: UInt64
    let layoutKey: String
    let followsBottom: Bool
    let jumpID: String?
    let bottomRequest: UInt64
    let content: (String) -> AnyView
    let scrollIntent: () -> Void
    let jumpCompleted: (String) -> Void
    func makeCoordinator() -> Coordinator { Coordinator(self) }
    func makeNSView(context: Context) -> NSScrollView {
        let scroll = NSScrollView()
        scroll.hasVerticalScroller = true; scroll.autohidesScrollers = true
        scroll.drawsBackground = true; scroll.backgroundColor = .windowBackgroundColor
        let table = NSTableView()
        table.headerView = nil; table.backgroundColor = .windowBackgroundColor
        table.style = .fullWidth
        table.selectionHighlightStyle = .none; table.intercellSpacing = .zero
        table.usesAutomaticRowHeights = false; table.rowHeight = 44
        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("message"))
        column.resizingMask = .autoresizingMask; table.addTableColumn(column)
        table.columnAutoresizingStyle = .uniformColumnAutoresizingStyle
        table.dataSource = context.coordinator; table.delegate = context.coordinator
        scroll.documentView = table
        context.coordinator.attach(scroll, table: table)
        return scroll
    }
    func updateNSView(_ scroll: NSScrollView, context: Context) { context.coordinator.enqueueUpdate(self) }
    @MainActor final class Coordinator: NSObject, NSTableViewDataSource, NSTableViewDelegate {
        var parent: NativeTranscriptTable
        weak var scroll: NSScrollView?
        weak var table: NSTableView?
        var renderedIDs: [String] = []
        var revision: UInt64?
        var layoutKey = ""
        var lastBottomRequest: UInt64?
        var lastJump: String?
        private var readingInputActive = false
        var inputMonitor: Any?
        private var widthObservation: NSObjectProtocol?
        private var configuredColumnWidth: CGFloat?
        var pendingLayout = false
        var readingAnchor: (id: String, offset: CGFloat)?
        struct Height {
            var width: CGFloat
            var generation: UInt64
            var content: NativeTranscriptContentRevision
            var value: CGFloat
        }
        struct Request {
            var width: CGFloat
            var generation: UInt64
            var content: NativeTranscriptContentRevision
        }
        private var heights: [String: Height] = [:]
        private var requests: [String: Request] = [:]
        private var nextGeneration: UInt64 = 0
        var changedHeightIDs: Set<String> = []
        var heightCommitScheduled = false
        private var queuedParent: NativeTranscriptTable?
        private var updateScheduled = false
        func enqueueUpdate(_ value: NativeTranscriptTable) {
            queuedParent = value
            guard !updateScheduled else { return }
            updateScheduled = true
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                self.updateScheduled = false
                guard let value = self.queuedParent else { return }
                self.queuedParent = nil
                self.update(value)
            }
        }
        func captureReadingAnchor() {
            guard readingAnchor == nil, (!parent.followsBottom || readingInputActive), (parent.jumpID == nil || readingInputActive), let scroll, let table else { return }
            let row = table.row(at: NSPoint(x: 0, y: scroll.contentView.bounds.minY))
            if renderedIDs.indices.contains(row) {
                readingAnchor = (renderedIDs[row], scroll.contentView.bounds.minY - table.rect(ofRow: row).minY)
            }
        }
        func heightChanged(id: String, generation: UInt64, width: CGFloat, height: CGFloat) {
            guard let request = requests[id], request.generation == generation, request.width == width,
                  renderedIDs.contains(id) else { return }
            let changed = heights[id]?.value != height || heights[id]?.generation != generation
            if changed { captureReadingAnchor() }
            heights[id] = Height(width: width, generation: generation, content: request.content, value: height)
            guard changed else { scheduleLayout(); return }
            changedHeightIDs.insert(id)
            guard !heightCommitScheduled else { return }
            heightCommitScheduled = true
            DispatchQueue.main.async { [weak self] in
                guard let self, let table = self.table else { return }
                self.heightCommitScheduled = false
                if self.readingAnchor == nil { self.captureReadingAnchor() }
                let changes = IndexSet(self.changedHeightIDs.compactMap { self.renderedIDs.firstIndex(of: $0) }); self.changedHeightIDs = []
                table.noteHeightOfRows(withIndexesChanged: changes)
                // Commit the native document geometry before clamping a target
                // against it; a new row rect can precede the scrollable extent.
                self.scroll?.layoutSubtreeIfNeeded()
                self.scheduleLayout()
            }
        }
        private func configure(_ cell: Cell, id: String, width: CGFloat) {
            guard let content = parent.contentRevisions[id] else { return }
            let contentWidth = width - NativeTranscriptTable.contentInset * 2
            if requests[id]?.width != contentWidth || requests[id]?.content != content {
                nextGeneration &+= 1
                requests[id] = Request(width: contentWidth, generation: nextGeneration, content: content)
            }
            guard let request = requests[id] else { return }
            cell.configure(id: id, generation: request.generation, content: parent.content(id), width: width)
            cell.widthChanged = { [weak self, weak cell] in
                guard let self, let cell, cell.representedID == id, let table = self.table,
                      let index = self.renderedIDs.firstIndex(of: id) else { return }
                self.configure(cell, id: id, width: table.frameOfCell(atColumn: 0, row: index).width)
            }
            cell.heightChanged = { [weak self] id, generation, width, height in
                self?.heightChanged(id: id, generation: generation, width: width, height: height)
            }
        }
        private func currentHeight(_ id: String) -> Height? {
            guard let height = heights[id], height.content == parent.contentRevisions[id],
                  height.width == (table?.tableColumns.first?.width ?? 0) - NativeTranscriptTable.contentInset * 2 else { return nil }
            return height
        }
        func tableView(_ tableView: NSTableView, heightOfRow row: Int) -> CGFloat {
            guard renderedIDs.indices.contains(row), let height = currentHeight(renderedIDs[row]) else { return tableView.rowHeight }
            return height.value + NativeTranscriptTable.contentInset
        }
        init(_ parent: NativeTranscriptTable) { self.parent = parent }
        func attach(_ scroll: NSScrollView, table: NSTableView) {
            self.scroll = scroll; self.table = table
            table.postsFrameChangedNotifications = true
            widthObservation = NotificationCenter.default.addObserver(forName: NSView.frameDidChangeNotification, object: table, queue: .main) { [weak self] _ in
                DispatchQueue.main.async { [weak self] in self?.refreshColumnWidth() }
            }
            inputMonitor = NSEvent.addLocalMonitorForEvents(matching: [.scrollWheel, .keyDown, .leftMouseDown]) { [weak self, weak scroll] event in
                guard let self, let scroll, event.window === scroll.window else { return event }
                let point = scroll.convert(event.locationInWindow, from: nil)
                let keyboard = (scroll.window?.firstResponder as? NSView)?.isDescendant(of: scroll) == true
                let key = event.type == .keyDown && keyboard && [UInt16(115), 116, 119, 121, 125, 126].contains(event.keyCode)
                let scroller = event.type == .leftMouseDown && scroll.verticalScroller?.frame.contains(point) == true
                if (event.type == .scrollWheel && scroll.bounds.contains(point)) || key || scroller {
                    self.beginReadingInput()
                }
                return event
            }
        }
        func beginReadingInput() {
            readingInputActive = true
            lastJump = nil
            readingAnchor = nil
            parent.scrollIntent()
        }
        func numberOfRows(in tableView: NSTableView) -> Int { renderedIDs.count }
        func tableView(_ tableView: NSTableView, shouldSelectRow row: Int) -> Bool { false }
        func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
            guard renderedIDs.indices.contains(row) else { return nil }
            let identifier = NSUserInterfaceItemIdentifier("message-cell")
            let cell = tableView.makeView(withIdentifier: identifier, owner: nil) as? Cell ?? Cell()
            cell.identifier = identifier
            configure(cell, id: renderedIDs[row], width: tableView.frameOfCell(atColumn: 0, row: row).width)
            return cell
        }
        func tableViewColumnDidResize(_ notification: Notification) { refreshColumnWidth() }
        private func refreshColumnWidth() {
            guard let width = table?.tableColumns.first?.width, width != configuredColumnWidth else { return }
            configuredColumnWidth = width
            captureReadingAnchor()
            refreshVisibleContent()
            if let table, !renderedIDs.isEmpty {
                table.noteHeightOfRows(withIndexesChanged: IndexSet(integersIn: 0..<renderedIDs.count))
            }
            scheduleLayout()
        }
        func refreshVisibleContent() {
            guard let table else { return }
            let range = table.rows(in: table.visibleRect)
            guard range.location != NSNotFound, range.length > 0 else { return }
            for index in range.location ..< min(range.location + range.length, renderedIDs.count) {
                if let cell = table.view(atColumn: 0, row: index, makeIfNecessary: false) as? Cell {
                    configure(cell, id: renderedIDs[index], width: table.frameOfCell(atColumn: 0, row: index).width)
                }
            }
        }
        func update(_ parent: NativeTranscriptTable) {
            if lastBottomRequest != parent.bottomRequest || (parent.jumpID != nil && parent.jumpID != self.parent.jumpID) {
                readingInputActive = false
                readingAnchor = nil
            }
            self.parent = parent
            guard let table else { return }
            refreshColumnWidth()
            let structural = renderedIDs != parent.ids
            let contentChanged = revision != parent.revision || layoutKey != parent.layoutKey
            if structural || contentChanged { captureReadingAnchor() }
            if structural {
                let previous = renderedIDs
                renderedIDs = parent.ids
                let retained = Set(renderedIDs)
                heights = heights.filter { retained.contains($0.key) }
                requests = requests.filter { retained.contains($0.key) }
                if parent.ids.starts(with: previous) {
                    table.insertRows(at: IndexSet(integersIn: previous.count ..< parent.ids.count), withAnimation: [])
                } else if previous.starts(with: parent.ids) {
                    table.removeRows(at: IndexSet(integersIn: parent.ids.count ..< previous.count), withAnimation: [])
                } else { table.reloadData() }
            }
            if contentChanged { refreshVisibleContent() }
            revision = parent.revision; layoutKey = parent.layoutKey
            if let anchor = readingAnchor, let index = renderedIDs.firstIndex(of: anchor.id) {
                table.scrollRowToVisible(index)
                table.layoutSubtreeIfNeeded()
            } else { readingAnchor = nil }
            if parent.jumpID == nil { lastJump = nil }
            if !readingInputActive, let id = parent.jumpID, id != lastJump, let index = renderedIDs.firstIndex(of: id) {
                lastJump = id
                table.scrollRowToVisible(index)
                table.layoutSubtreeIfNeeded()
                scheduleLayout()
            } else if structural || contentChanged || lastBottomRequest != parent.bottomRequest { scheduleLayout() }
            lastBottomRequest = parent.bottomRequest
        }
        private func visibleSizesPending() -> Bool {
            guard let table else { return true }
            let range = table.rows(in: table.visibleRect)
            guard range.location != NSNotFound else { return false }
            for index in range.location ..< min(range.location + range.length, renderedIDs.count) {
                guard let cell = table.view(atColumn: 0, row: index, makeIfNecessary: true) as? Cell else { return true }
                if cell.isHeightPending || currentHeight(renderedIDs[index]) == nil { return true }
            }
            return false
        }
        func scheduleLayout() {
            guard !pendingLayout else { return }
            pendingLayout = true
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                self.pendingLayout = false
                guard let table = self.table, let scroll = self.scroll else { return }
                let clip = scroll.contentView
                if let anchor = self.readingAnchor, let index = self.renderedIDs.firstIndex(of: anchor.id),
                   let cell = table.view(atColumn: 0, row: index, makeIfNecessary: true) as? Cell {
                    // A restored anchor can lie just outside the viewport after
                    // a preceding row changes height. Lay out that actual cell.
                    if cell.requiresContentLayout { cell.layoutSubtreeIfNeeded() }
                }
                if !self.readingInputActive, let id = self.parent.jumpID, self.lastJump == id, let index = self.renderedIDs.firstIndex(of: id), let cell = table.view(atColumn: 0, row: index, makeIfNecessary: false) as? Cell, !cell.isHeightPending, !self.heightCommitScheduled, !self.visibleSizesPending(), self.currentHeight(id) != nil {
                    let rect = table.rect(ofRow: index)
                    let desired = clip.constrainBoundsRect(NSRect(origin: NSPoint(x: clip.bounds.minX, y: rect.minY + NativeTranscriptTable.contentInset), size: clip.bounds.size))
                    clip.scroll(to: desired.origin); scroll.reflectScrolledClipView(clip)
                    self.parent.jumpCompleted(id)
                } else if let anchor = self.readingAnchor, let index = self.renderedIDs.firstIndex(of: anchor.id), let cell = table.view(atColumn: 0, row: index, makeIfNecessary: true) as? Cell, !cell.isHeightPending, !self.heightCommitScheduled, self.currentHeight(anchor.id) != nil {
                    let desired = clip.constrainBoundsRect(NSRect(origin: NSPoint(x: clip.bounds.minX, y: table.rect(ofRow: index).minY + anchor.offset), size: clip.bounds.size))
                    clip.scroll(to: desired.origin); scroll.reflectScrolledClipView(clip)
                    self.readingAnchor = nil
                } else if self.parent.followsBottom && !self.readingInputActive {
                    let desired = clip.constrainBoundsRect(NSRect(origin: NSPoint(x: clip.bounds.minX, y: table.bounds.maxY - clip.bounds.height), size: clip.bounds.size))
                    if abs(desired.minY - clip.bounds.minY) > 1 { clip.scroll(to: desired.origin); scroll.reflectScrolledClipView(clip) }
                }
            }
        }
        deinit { if let inputMonitor { NSEvent.removeMonitor(inputMonitor) }; if let widthObservation { NotificationCenter.default.removeObserver(widthObservation) } }
    }
    @MainActor final class Cell: NSTableCellView {
        private let host = TranscriptHostingView(rootView: AnyView(EmptyView()))
        private var content = AnyView(EmptyView())
        var heightChanged: ((String, UInt64, CGFloat, CGFloat) -> Void)?
        var widthChanged: (() -> Void)?
        private var widthNotificationScheduled = false
        private var generation: UInt64 = 0
        private(set) var representedID: String?
        private(set) var contentWidth: CGFloat = 0
        private(set) var measuredHeight: CGFloat = 0
        private var notificationScheduled = false
        private var awaitingContentLayout = false
        var isHeightPending: Bool { notificationScheduled || widthNotificationScheduled || awaitingContentLayout }
        var requiresContentLayout: Bool { awaitingContentLayout }
        override init(frame frameRect: NSRect) {
            super.init(frame: frameRect)
            host.sizingOptions = [.intrinsicContentSize]
            host.translatesAutoresizingMaskIntoConstraints = false
            addSubview(host)
            NSLayoutConstraint.activate([
                host.leadingAnchor.constraint(equalTo: leadingAnchor, constant: NativeTranscriptTable.contentInset),
                host.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -NativeTranscriptTable.contentInset),
                host.topAnchor.constraint(equalTo: topAnchor, constant: NativeTranscriptTable.contentInset),
                host.bottomAnchor.constraint(equalTo: bottomAnchor)
            ])
            host.sizeInvalidated = { [weak self] in self?.scheduleHeightNotification() }
            host.layoutCompleted = { [weak self] in
                guard let self else { return }
                self.awaitingContentLayout = false
                self.scheduleHeightNotification()
            }
        }
        override func layout() {
            super.layout()
            guard representedID != nil, bounds.width > NativeTranscriptTable.contentInset * 2,
                  bounds.width - NativeTranscriptTable.contentInset * 2 != contentWidth,
                  !widthNotificationScheduled else { return }
            widthNotificationScheduled = true
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                self.widthNotificationScheduled = false
                self.widthChanged?()
            }
        }
        convenience init() { self.init(frame: .zero) }
        required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }
        func configure(id: String, generation: UInt64, content: AnyView, width: CGFloat) {
            guard representedID != id || self.generation != generation else { return }
            representedID = id
            self.generation = generation
            self.content = content
            contentWidth = width - NativeTranscriptTable.contentInset * 2
            guard contentWidth > 0 else { return }
            // The delegate returns this exact body at the actual single-column
            // width. Do not let its initial intrinsic query see empty/old content.
            awaitingContentLayout = true
            host.rootView = AnyView(content.frame(width: contentWidth).fixedSize(horizontal: false, vertical: true))
            host.needsLayout = true
        }
        private func scheduleHeightNotification() {
            guard !notificationScheduled, !awaitingContentLayout else { return }
            notificationScheduled = true
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                // Intrinsic reads may synchronously invalidate the same host.
                // Actual later native layout remains a separate invalidation.
                defer { self.notificationScheduled = false }
                guard !self.awaitingContentLayout else { return }
                let generation = self.generation
                let scale = self.window?.backingScaleFactor ?? 1
                let height = ceil(self.host.intrinsicContentSize.height * scale) / scale
                guard height > 0, let id = self.representedID, self.generation == generation else { return }
                self.measuredHeight = height
                self.heightChanged?(id, generation, self.contentWidth, height)
            }
        }
    }
}

@MainActor private final class TranscriptHostingView: NSHostingView<AnyView> {
    var sizeInvalidated: (() -> Void)?
    var layoutCompleted: (() -> Void)?
    override func invalidateIntrinsicContentSize() {
        super.invalidateIntrinsicContentSize()
        sizeInvalidated?()
    }
    override func layout() {
        super.layout()
        // SwiftUI local view state also changes intrinsic size through layout.
        layoutCompleted?()
    }
}
