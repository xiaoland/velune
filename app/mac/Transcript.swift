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
    @State private var userScrolling = false
    @State private var outline = TranscriptOutlineState()
    @State private var jumpTarget: String?
    @StateObject private var scrollObserver = TranscriptScrollObserver()
    @State private var expandedWork: Set<String> = []
    private var visibleRows: [TranscriptRow] { presentation == .userOutline ? outline.visibleRows(model.rows, userIDs: Set(model.userRows.map(\.id))) : model.rows }
    var body: some View {
        ScrollViewReader { proxy in
            List {
                ForEach(model.items(for: visibleRows), id: \.scrollID) { item in
                    Group { switch item {
                    case .message(let row): message(row, proxy: proxy)
                    case .work(let turn, let rows):
                        InlineMessageDisclosure(isExpanded: workExpansion(turn.id), accessibilityLabel: workLabel(turn)) {
                            LazyVStack(alignment: .leading, spacing: 24) {
                                ForEach(rows) { row in ConversationMessageView(row: row, cwd: cwd).id(row.id) }
                            }
                        } label: { Text(workLabel(turn)).font(.callout).foregroundStyle(.secondary) }
                    } }
                    .frame(maxWidth: 760).frame(maxWidth: .infinity)
                    .listRowInsets(EdgeInsets(top: 24, leading: 24, bottom: 0, trailing: 24))
                    .listRowSeparator(.hidden).listRowBackground(Color.clear)
                    .selectionDisabled()
                }
                if let activity { HStack { ProgressView().controlSize(.small); Text(activity).foregroundStyle(.secondary) }.font(.callout) }
                Color.clear.frame(height: 1).id("transcript-bottom")
                    .listRowSeparator(.hidden).listRowBackground(Color.clear)
            }
            .listStyle(.plain)
            .overlay(alignment: .bottomTrailing) {
                HStack(spacing: 8) {
                    if !following {
                        Button {
                            following = true
                            scrollToBottom(proxy)
                        } label: { Image(systemName: "arrow.down") }
                            .help("回到底部").accessibilityLabel("回到底部")
                    }
                }.buttonStyle(.bordered).padding(16)
            }
            .sheet(isPresented: $showsOutline) {
                TranscriptOutlineView(model: model) { id in
                    following = false
                    if presentation == .userOutline { _ = outline.expand(from: id, in: model.rows) }
                    jump(to: id, proxy: proxy)
                }
            }
            .onAppear { following = true; scrollToBottom(proxy) }
            .onChange(of: conversationID) { _, _ in outline = TranscriptOutlineState(); expandedWork = []; following = true; scrollToBottom(proxy) }
            .onChange(of: presentation) { _, _ in outline = TranscriptOutlineState(); following = true; scrollToBottom(proxy) }
            .onChange(of: scrollRequest) { _, _ in
                if presentation == .userOutline { outline.followSentUser(after: sentAfterUserID, in: model.rows) }
                following = true; scrollToBottom(proxy)
            }
            .onChange(of: model.contentRevision) { _, _ in
                expandedWork = model.retainedExpandedTurnIDs(expandedWork)
                if outline.reconcile(model.rows) { following = false }
                if following && !userScrolling { scrollToBottom(proxy) }
            }
        }
    }
    @ViewBuilder private func message(_ row: TranscriptRow, proxy: ScrollViewProxy) -> some View {
        let followingBinding = $following
        let scrollingBinding = $userScrolling
        let targetBinding = $jumpTarget
        let observer = scrollObserver
        let messageID = row.id
        Group {
        if presentation == .userOutline && row.message.role == .user {
            Button {
                following = false
                if outline.expandedFromUserID == row.id { outline.collapse() }
                else { _ = outline.expand(from: row.id, in: model.rows) }
                jump(to: row.id, proxy: proxy)
            } label: { ConversationMessageView(row: row, cwd: cwd) }
                .buttonStyle(.plain).help(outline.expandedFromUserID == row.id ? "返回用户消息大纲" : "展开从此消息开始的会话")
        } else { ConversationMessageView(row: row, cwd: cwd) }
        }.background {
            TranscriptRowLayout(requested: jumpTarget == row.id, registered: { scroll in
                observer.observe(scroll) { scrolling, nearBottom in
                    scrollingBinding.wrappedValue = scrolling
                    if scrolling { targetBinding.wrappedValue = nil }
                    followingBinding.wrappedValue = nearBottom
                }
            }) { if targetBinding.wrappedValue == messageID { targetBinding.wrappedValue = nil } }
        }
    }
    private func jump(to id: String, proxy: ScrollViewProxy) {
        jumpTarget = id
        DispatchQueue.main.async { proxy.scrollTo(id, anchor: .top) }
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
    private func scrollToBottom(_ proxy: ScrollViewProxy) {
        jumpTarget = nil
        DispatchQueue.main.async { proxy.scrollTo("transcript-bottom", anchor: .bottom) }
    }
}

/// Native live-scroll notifications distinguish reading history from programmatic
/// scrolling and content layout changes (including streamed Markdown and images).
@MainActor private final class TranscriptScrollObserver: ObservableObject {
    private weak var observedScroll: NSScrollView?
    private var observations: [NSObjectProtocol] = []
    private var changed: ((Bool, Bool) -> Void)?
    func observe(_ scroll: NSScrollView, changed: @escaping (Bool, Bool) -> Void) {
        self.changed = changed
        guard scroll !== observedScroll else { return }
        observations.forEach(NotificationCenter.default.removeObserver)
        observations = []; observedScroll = scroll
        for (name, scrolling) in [(NSScrollView.willStartLiveScrollNotification, true), (NSScrollView.didLiveScrollNotification, true), (NSScrollView.didEndLiveScrollNotification, false)] {
            observations.append(NotificationCenter.default.addObserver(forName: name, object: scroll, queue: .main) { [weak self, weak scroll] _ in
                guard let self, let scroll, let document = scroll.documentView else { return }
                let bounds = scroll.contentView.bounds
                let remaining = document.isFlipped ? document.bounds.maxY - bounds.maxY : bounds.minY - document.bounds.minY
                MainActor.assumeIsolated { self.changed?(scrolling, remaining <= 48) }
            })
        }
    }
    deinit { observations.forEach(NotificationCenter.default.removeObserver) }
}

@MainActor private extension TranscriptModel.Item {
    var scrollID: String {
        switch self {
        case .message(let row): return row.id
        case .work(let turn, _): return "work:" + turn.id
        }
    }
}

/// Actual message rows identify their owning native scroll container. Only the
/// requested row corrects its anchor from an observed, completed layout frame.
private struct TranscriptRowLayout: NSViewRepresentable {
    let requested: Bool
    let registered: (NSScrollView) -> Void
    let completed: () -> Void
    func makeNSView(context: Context) -> Probe { Probe() }
    func sizeThatFits(_ proposal: ProposedViewSize, nsView: Probe, context: Context) -> CGSize? {
        CGSize(width: proposal.width ?? 0, height: proposal.height ?? 0)
    }
    func updateNSView(_ view: Probe, context: Context) {
        if requested != view.requested { view.done = false }
        view.requested = requested; view.registered = registered; view.completed = completed
        view.attach(); view.schedule()
    }
    final class Probe: NSView {
        var requested = false
        var done = false
        var registered: ((NSScrollView) -> Void)?
        var completed: (() -> Void)?
        private var observations: [NSObjectProtocol] = []
        private weak var scroll: NSScrollView?
        private var scheduled = false
        private var correcting = false
        required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }
        init() { super.init(frame: .zero) }
        override func viewDidMoveToWindow() { super.viewDidMoveToWindow(); attach(); schedule() }
        override func viewDidMoveToSuperview() { super.viewDidMoveToSuperview(); attach(); schedule() }
        override func layout() { super.layout(); schedule() }
        func attach() {
            guard let found = enclosingScrollView, found !== scroll, let document = found.documentView else { return }
            observations.forEach(NotificationCenter.default.removeObserver)
            observations = []; scroll = found
            DispatchQueue.main.async { [weak self, weak found] in
                guard let self, let found, self.window != nil else { return }
                self.registered?(found)
            }
            document.postsFrameChangedNotifications = true
            found.contentView.postsBoundsChangedNotifications = true
            for (name, object) in [(NSView.frameDidChangeNotification, document), (NSView.boundsDidChangeNotification, found.contentView)] {
                observations.append(NotificationCenter.default.addObserver(forName: name, object: object, queue: .main) { [weak self] _ in
                    guard let self, !self.correcting else { return }
                    self.schedule()
                })
            }
        }
        func schedule() {
            guard requested, !done, !scheduled, !correcting else { return }
            scheduled = true
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                self.align(); self.scheduled = false
            }
        }
        private func align() {
            guard requested, !done, let scroll, let document = scroll.documentView, let window else { return }
            let frame = convert(bounds, to: document)
            let clip = scroll.contentView
            let visible = document.visibleRect.intersection(document.convert(window.contentLayoutRect, from: nil))
            guard !visible.isEmpty, frame.height > 0 else { return }
            let y = document.isFlipped ? clip.bounds.minY + frame.minY - visible.minY : clip.bounds.minY + frame.maxY - visible.maxY
            let desired = clip.constrainBoundsRect(NSRect(origin: NSPoint(x: clip.bounds.minX, y: y), size: clip.bounds.size))
            correcting = true
            if abs(desired.minY - clip.bounds.minY) > 1 {
                clip.scroll(to: desired.origin); scroll.reflectScrolledClipView(clip)
            }
            correcting = false
            done = true; completed?()
        }
        deinit { observations.forEach(NotificationCenter.default.removeObserver) }
    }
}
