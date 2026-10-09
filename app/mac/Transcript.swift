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
        let items = model.items(for: visibleRows)
        let ids = items.map(\.scrollID) + (activity == nil ? [] : ["transcript-activity"])
        NativeTranscriptTable(ids: ids, revision: model.contentRevision,
            layoutKey: expandedWork.sorted().joined(separator: "|") + String(describing: presentation) + (activity ?? ""),
            followsBottom: following, jumpID: jumpTarget, bottomRequest: bottomRequest,
            content: { id in
                if id == "transcript-activity", let activity { return AnyView(HStack { ProgressView().controlSize(.small); Text(activity).foregroundStyle(.secondary) }.font(.callout)) }
                guard let item = items.first(where: { $0.scrollID == id }) else { return AnyView(EmptyView()) }
                return AnyView(itemView(item))
            }, scrollIntent: { following = false; jumpTarget = nil },
            scrollEnded: { nearBottom in following = nearBottom },
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
            case .work(let turn, let rows):
                InlineMessageDisclosure(isExpanded: workExpansion(turn.id), accessibilityLabel: workLabel(turn)) {
                    VStack(alignment: .leading, spacing: 24) {
                        ForEach(rows) { row in ConversationMessageView(row: row, cwd: cwd).id(row.id) }
                    }
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
private struct NativeTranscriptTable: NSViewRepresentable {
    private static let contentInset: CGFloat = 24
    let ids: [String]
    let revision: UInt64
    let layoutKey: String
    let followsBottom: Bool
    let jumpID: String?
    let bottomRequest: UInt64
    let content: (String) -> AnyView
    let scrollIntent: () -> Void
    let scrollEnded: (Bool) -> Void
    let jumpCompleted: (String) -> Void
    func makeCoordinator() -> Coordinator { Coordinator(self) }
    func makeNSView(context: Context) -> NSScrollView {
        let scroll = NSScrollView()
        scroll.hasVerticalScroller = true; scroll.autohidesScrollers = true
        scroll.drawsBackground = true; scroll.backgroundColor = .windowBackgroundColor
        let table = NSTableView()
        table.headerView = nil; table.backgroundColor = .windowBackgroundColor
        table.selectionHighlightStyle = .none; table.intercellSpacing = .zero
        table.usesAutomaticRowHeights = true; table.rowHeight = 44
        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("message"))
        column.resizingMask = .autoresizingMask; table.addTableColumn(column)
        table.columnAutoresizingStyle = .uniformColumnAutoresizingStyle
        table.dataSource = context.coordinator; table.delegate = context.coordinator
        scroll.documentView = table
        context.coordinator.attach(scroll, table: table)
        return scroll
    }
    func updateNSView(_ scroll: NSScrollView, context: Context) { context.coordinator.update(self) }
    @MainActor final class Coordinator: NSObject, NSTableViewDataSource, NSTableViewDelegate {
        var parent: NativeTranscriptTable
        weak var scroll: NSScrollView?
        weak var table: NSTableView?
        var renderedIDs: [String] = []
        var revision: UInt64?
        var layoutKey = ""
        var lastBottomRequest: UInt64?
        var lastJump: String?
        var inputMonitor: Any?
        var observations: [NSObjectProtocol] = []
        var pendingLayout = false
        var readingAnchor: (id: String, offset: CGFloat)?
        init(_ parent: NativeTranscriptTable) { self.parent = parent }
        func attach(_ scroll: NSScrollView, table: NSTableView) {
            self.scroll = scroll; self.table = table
            inputMonitor = NSEvent.addLocalMonitorForEvents(matching: [.scrollWheel, .keyDown, .leftMouseDown]) { [weak self, weak scroll] event in
                guard let self, let scroll, event.window === scroll.window else { return event }
                let point = scroll.convert(event.locationInWindow, from: nil)
                let keyboard = (scroll.window?.firstResponder as? NSView)?.isDescendant(of: scroll) == true
                let key = event.type == .keyDown && keyboard && [UInt16(115), 116, 119, 121, 125, 126].contains(event.keyCode)
                let scroller = event.type == .leftMouseDown && scroll.verticalScroller?.frame.contains(point) == true
                if (event.type == .scrollWheel && scroll.bounds.contains(point)) || key || scroller {
                    self.lastJump = nil; self.parent.scrollIntent()
                }
                return event
            }
            observations.append(NotificationCenter.default.addObserver(forName: NSScrollView.didEndLiveScrollNotification, object: scroll, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { guard let self else { return }; self.parent.scrollEnded(self.nearBottom) }
            })
            table.postsFrameChangedNotifications = true
            observations.append(NotificationCenter.default.addObserver(forName: NSView.frameDidChangeNotification, object: table, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.scheduleLayout() }
            })
        }
        var nearBottom: Bool {
            guard let scroll, let table else { return true }
            return table.bounds.maxY - scroll.contentView.bounds.maxY <= 48
        }
        func numberOfRows(in tableView: NSTableView) -> Int { renderedIDs.count }
        func tableView(_ tableView: NSTableView, shouldSelectRow row: Int) -> Bool { false }
        func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
            guard renderedIDs.indices.contains(row) else { return nil }
            let identifier = NSUserInterfaceItemIdentifier("message-cell")
            let cell = tableView.makeView(withIdentifier: identifier, owner: nil) as? Cell ?? Cell()
            cell.identifier = identifier
            cell.configure(id: renderedIDs[row], content: parent.content(renderedIDs[row]))
            cell.heightChanged = { [weak self, weak cell] in
                guard let self, let table = self.table, let cell else { return }
                let index = table.row(for: cell)
                guard index >= 0 else { return }
                table.noteHeightOfRows(withIndexesChanged: IndexSet(integer: index))
                self.scheduleLayout()
            }
            return cell
        }
        func update(_ parent: NativeTranscriptTable) {
            self.parent = parent
            guard let table else { return }
            let structural = renderedIDs != parent.ids
            let contentChanged = revision != parent.revision || layoutKey != parent.layoutKey
            if structural {
                if !parent.followsBottom, parent.jumpID == nil, let scroll {
                    let row = table.row(at: NSPoint(x: 0, y: scroll.contentView.bounds.minY))
                    if renderedIDs.indices.contains(row) {
                        readingAnchor = (renderedIDs[row], scroll.contentView.bounds.minY - table.rect(ofRow: row).minY)
                    }
                }
                let previous = renderedIDs
                renderedIDs = parent.ids
                if parent.ids.starts(with: previous) {
                    table.insertRows(at: IndexSet(integersIn: previous.count ..< parent.ids.count), withAnimation: [])
                } else if previous.starts(with: parent.ids) {
                    table.removeRows(at: IndexSet(integersIn: parent.ids.count ..< previous.count), withAnimation: [])
                } else { table.reloadData() }
            }
            if contentChanged {
                let range = table.rows(in: table.visibleRect)
                if range.location != NSNotFound, range.length > 0 {
                    for index in range.location ..< min(range.location + range.length, renderedIDs.count) {
                        (table.view(atColumn: 0, row: index, makeIfNecessary: false) as? Cell)?.configure(id: renderedIDs[index], content: parent.content(renderedIDs[index]))
                    }
                    table.noteHeightOfRows(withIndexesChanged: IndexSet(integersIn: range.location ..< min(range.location + range.length, renderedIDs.count)))
                }
            }
            revision = parent.revision; layoutKey = parent.layoutKey
            if let anchor = readingAnchor, let index = renderedIDs.firstIndex(of: anchor.id) {
                table.scrollRowToVisible(index)
                table.layoutSubtreeIfNeeded()
            } else { readingAnchor = nil }
            if parent.jumpID == nil { lastJump = nil }
            if let id = parent.jumpID, id != lastJump, let index = renderedIDs.firstIndex(of: id) {
                lastJump = id
                table.scrollRowToVisible(index)
                table.layoutSubtreeIfNeeded()
                scheduleLayout()
            } else if structural || contentChanged || lastBottomRequest != parent.bottomRequest { scheduleLayout() }
            lastBottomRequest = parent.bottomRequest
        }
        func scheduleLayout() {
            guard !pendingLayout else { return }
            pendingLayout = true
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                self.pendingLayout = false
                guard let table = self.table, let scroll = self.scroll else { return }
                let clip = scroll.contentView
                if let id = self.parent.jumpID, self.lastJump == id, let index = self.renderedIDs.firstIndex(of: id), table.view(atColumn: 0, row: index, makeIfNecessary: false) != nil {
                    let rect = table.rect(ofRow: index)
                    let desired = clip.constrainBoundsRect(NSRect(origin: NSPoint(x: clip.bounds.minX, y: rect.minY + NativeTranscriptTable.contentInset), size: clip.bounds.size))
                    clip.scroll(to: desired.origin); scroll.reflectScrolledClipView(clip)
                    self.parent.jumpCompleted(id)
                } else if let anchor = self.readingAnchor, let index = self.renderedIDs.firstIndex(of: anchor.id), table.view(atColumn: 0, row: index, makeIfNecessary: false) != nil {
                    let desired = clip.constrainBoundsRect(NSRect(origin: NSPoint(x: clip.bounds.minX, y: table.rect(ofRow: index).minY + anchor.offset), size: clip.bounds.size))
                    clip.scroll(to: desired.origin); scroll.reflectScrolledClipView(clip)
                    self.readingAnchor = nil
                } else if self.parent.followsBottom {
                    let desired = clip.constrainBoundsRect(NSRect(origin: NSPoint(x: clip.bounds.minX, y: table.bounds.maxY - clip.bounds.height), size: clip.bounds.size))
                    if abs(desired.minY - clip.bounds.minY) > 1 { clip.scroll(to: desired.origin); scroll.reflectScrolledClipView(clip) }
                }
            }
        }
        deinit { if let inputMonitor { NSEvent.removeMonitor(inputMonitor) }; observations.forEach(NotificationCenter.default.removeObserver) }
    }
    @MainActor final class Cell: NSTableCellView {
        let controller = NSHostingController(rootView: AnyView(EmptyView()))
        private var host: NSView { controller.view }
        private var contentHeight: NSLayoutConstraint?
        var heightChanged: (() -> Void)?
        private var representedID: String?
        private var lastHeight: CGFloat = 0
        private var notifying = false
        override init(frame frameRect: NSRect) {
            super.init(frame: frameRect)
            controller.sizingOptions = []
            host.translatesAutoresizingMaskIntoConstraints = false
            addSubview(host)
            let height = host.heightAnchor.constraint(equalToConstant: 1)
            height.isActive = true; contentHeight = height
            NSLayoutConstraint.activate([host.leadingAnchor.constraint(equalTo: leadingAnchor, constant: NativeTranscriptTable.contentInset), host.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -NativeTranscriptTable.contentInset), host.topAnchor.constraint(equalTo: topAnchor, constant: NativeTranscriptTable.contentInset), host.bottomAnchor.constraint(equalTo: bottomAnchor)])
        }
        convenience init() { self.init(frame: .zero) }
        required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }
        func configure(id: String, content: AnyView) {
            if representedID != id { lastHeight = 0; representedID = id }
            controller.rootView = content
            needsLayout = true
        }
        override func layout() {
            super.layout()
            let width = bounds.width - NativeTranscriptTable.contentInset * 2
            guard width > 0 else { return }
            let height = controller.sizeThatFits(in: CGSize(width: width, height: .greatestFiniteMagnitude)).height
            guard height > 0, abs(height - lastHeight) > 1, !notifying else { return }
            lastHeight = height; contentHeight?.constant = height; notifying = true
            DispatchQueue.main.async { [weak self] in guard let self else { return }; self.notifying = false; self.heightChanged?() }
        }
    }
}
