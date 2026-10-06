import SwiftUI
import AppKit
import VeluneBindings

struct TranscriptView: View {
    @ObservedObject var model: TranscriptModel
    let conversationID: String?
    let scrollRequest: UInt64
    let sentAfterUserID: String?
    let activity: String?
    let presentation: BindingTranscriptPresentation
    @State private var following = true
    @State private var userScrolling = false
    @State private var showsOutline = false
    @State private var outline = TranscriptOutlineState()
    @State private var expandedWork: Set<String> = []
    private var visibleRows: [TranscriptRow] { presentation == .userOutline ? outline.visibleRows(model.rows) : model.rows }
    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 24) {
                    ForEach(model.items(for: visibleRows)) { item in
                        switch item {
                        case .message(let row): message(row, proxy: proxy)
                        case .work(let turn, let rows):
                            ImmediateDisclosureGroup(isExpanded: workExpansion(turn.id)) {
                                LazyVStack(alignment: .leading, spacing: 24) {
                                    ForEach(rows) { row in ConversationMessageView(row: row).id(row.id) }
                                }.padding(.top, 12)
                            } label: { Text(workLabel(turn)).font(.callout).foregroundStyle(.secondary) }
                        }
                    }
                    if let activity { HStack { ProgressView().controlSize(.small); Text(activity).foregroundStyle(.secondary) }.font(.callout) }
                    Color.clear.frame(height: 1).id("transcript-bottom")
                }
                .frame(maxWidth: 760).padding(24).frame(maxWidth: .infinity)
                .background(GeometryReader { geometry in Color.clear.preference(key: TranscriptHeight.self, value: geometry.size.height) })
                .background(TranscriptScrollObserver { scrolling, nearBottom in
                    userScrolling = scrolling
                    following = nearBottom
                })
            }
            .overlay(alignment: .bottomTrailing) {
                HStack(spacing: 8) {
                    if !following {
                        Button {
                            following = true
                            proxy.scrollTo("transcript-bottom", anchor: .bottom)
                        } label: { Image(systemName: "arrow.down") }
                            .help("回到底部").accessibilityLabel("回到底部")
                    }
                    if !model.userRows.isEmpty {
                        Button { showsOutline = true } label: { Image(systemName: "list.bullet") }
                            .help("会话大纲").accessibilityLabel("会话大纲")
                    }
                }.buttonStyle(.bordered).padding(16)
            }
            .sheet(isPresented: $showsOutline) {
                TranscriptOutlineView(model: model) { id in
                    following = false
                    if presentation == .userOutline { _ = outline.expand(from: id, in: model.rows) }
                    DispatchQueue.main.async { proxy.scrollTo(id, anchor: .top) }
                }
            }
            .onAppear { following = true; scrollToBottom(proxy) }
            .onChange(of: conversationID) { _, _ in outline = TranscriptOutlineState(); expandedWork = []; following = true; scrollToBottom(proxy) }
            .onChange(of: presentation) { _, _ in outline = TranscriptOutlineState(); following = true; scrollToBottom(proxy) }
             .onChange(of: scrollRequest) { _, _ in
                if presentation == .userOutline { outline.followSentUser(after: sentAfterUserID.map { model.confirmedMessageIDs[$0] ?? $0 }, in: model.rows) }
                following = true; scrollToBottom(proxy)
            }
             .onChange(of: model.contentRevision) { _, _ in
                expandedWork = model.retainedExpandedTurnIDs(expandedWork)
                outline.confirmIdentities(model.confirmedMessageIDs)
                if outline.reconcile(model.rows) { following = false }
                if following && !userScrolling { scrollToBottom(proxy) }
            }
            .onPreferenceChange(TranscriptHeight.self) { _ in if following && !userScrolling { scrollToBottom(proxy) } }
        }
    }
    @ViewBuilder private func message(_ row: TranscriptRow, proxy: ScrollViewProxy) -> some View {
        if presentation == .userOutline && row.message.role == .user {
            Button {
                following = false
                if outline.expandedFromUserID == row.id { outline.collapse() }
                else { _ = outline.expand(from: row.id, in: model.rows) }
                DispatchQueue.main.async { proxy.scrollTo(row.id, anchor: .top) }
            } label: { ConversationMessageView(row: row) }
                .buttonStyle(.plain).help(outline.expandedFromUserID == row.id ? "返回用户消息大纲" : "展开从此消息开始的会话").id(row.id)
        } else { ConversationMessageView(row: row).id(row.id) }
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
        // Wait for the new Markdown/Disclosure layout before moving the anchor.
        DispatchQueue.main.async { proxy.scrollTo("transcript-bottom", anchor: .bottom) }
    }
}
private struct TranscriptHeight: PreferenceKey {
    static let defaultValue: CGFloat = 0
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) { value = nextValue() }
}

/// Native live-scroll notifications distinguish reading history from programmatic
/// scrolling and content layout changes (including streamed Markdown and images).
private struct TranscriptScrollObserver: NSViewRepresentable {
    let changed: (Bool, Bool) -> Void
    func makeNSView(context: Context) -> Probe { Probe(changed: changed) }
    func updateNSView(_ view: Probe, context: Context) { view.changed = changed; view.attach() }
    final class Probe: NSView {
        var changed: (Bool, Bool) -> Void
        private weak var observedScroll: NSScrollView?
        private var observations: [NSObjectProtocol] = []
        init(changed: @escaping (Bool, Bool) -> Void) { self.changed = changed; super.init(frame: .zero) }
        required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }
        override func viewDidMoveToWindow() { super.viewDidMoveToWindow(); attach() }
        override func viewDidMoveToSuperview() { super.viewDidMoveToSuperview(); attach() }
        func attach() {
            guard let scroll = enclosingScrollView, scroll !== observedScroll else { return }
            observations.forEach(NotificationCenter.default.removeObserver)
            observations = []; observedScroll = scroll
            for (name, scrolling) in [(NSScrollView.willStartLiveScrollNotification, true), (NSScrollView.didLiveScrollNotification, true), (NSScrollView.didEndLiveScrollNotification, false)] {
                observations.append(NotificationCenter.default.addObserver(forName: name, object: scroll, queue: .main) { [weak self, weak scroll] _ in
                    guard let self, let scroll, let document = scroll.documentView else { return }
                    let bounds = scroll.contentView.bounds
                    let remaining = document.isFlipped ? document.bounds.maxY - bounds.maxY : bounds.minY - document.bounds.minY
                    self.changed(scrolling, remaining <= 48)
                })
            }
        }
        deinit { observations.forEach(NotificationCenter.default.removeObserver) }
    }
}
