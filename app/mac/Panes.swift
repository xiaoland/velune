import SwiftUI

/// A window's panes as arranged: a pane, or a split laid out by its fractions.
struct PaneLayoutView: View {
    @ObservedObject var window: ConversationWindowModel
    let layout: PaneLayout
    /// Child indices from the root to this part of the layout.
    let path: [Int]
    let previewEmpty: Bool

    var body: some View {
        switch layout {
        case .pane(let id):
            if let pane = window.panes[id] {
                ConversationPaneView(window: window, pane: pane, store: pane.store, previewEmpty: previewEmpty).id(id)
            }
        case .split(let split):
            PaneSplitView(window: window, split: split, path: path, previewEmpty: previewEmpty)
        }
    }
}

/// Panes side by side or stacked, with dividers that resize the two panes
/// beside them.
private struct PaneSplitView: View {
    @ObservedObject var window: ConversationWindowModel
    let split: PaneSplit
    let path: [Int]
    let previewEmpty: Bool
    /// The fractions when the divider being dragged was picked up.
    @State private var dragStart: [Double]?
    private static let minimumLength: CGFloat = 280

    var body: some View {
        GeometryReader { geometry in
            let horizontal = split.axis == .horizontal
            let length = (horizontal ? geometry.size.width : geometry.size.height) - CGFloat(split.children.count - 1)
            let stack = horizontal ? AnyLayout(HStackLayout(spacing: 0)) : AnyLayout(VStackLayout(spacing: 0))
            stack {
                ForEach(Array(split.children.enumerated()), id: \.element.paneIDs) { index, child in
                    if index > 0 { divider(before: index, length: length) }
                    // Recursion through a type-erased view: a split can hold splits.
                    AnyView(PaneLayoutView(window: window, layout: child, path: path + [index], previewEmpty: previewEmpty))
                        .frame(width: horizontal ? max(0, length * split.fractions[index]) : geometry.size.width,
                               height: horizontal ? geometry.size.height : max(0, length * split.fractions[index]))
                        .clipped()
                }
            }
        }
    }

    private func divider(before index: Int, length: CGFloat) -> some View {
        let horizontal = split.axis == .horizontal
        return Divider()
            .overlay {
                // A wider grip than the line it moves.
                Color.clear
                    .frame(width: horizontal ? 7 : nil, height: horizontal ? nil : 7)
                    .contentShape(Rectangle())
                    .pointerStyle(horizontal ? .columnResize : .rowResize)
                    .gesture(DragGesture(minimumDistance: 1, coordinateSpace: .global)
                        .onChanged { value in
                            let start = dragStart ?? split.fractions
                            if dragStart == nil { dragStart = start }
                            guard length > 0 else { return }
                            let pair = start[index - 1] + start[index]
                            let minimum = min(pair / 2, Double(Self.minimumLength / length))
                            let moved = Double((horizontal ? value.translation.width : value.translation.height) / length)
                            var fractions = start
                            fractions[index - 1] = min(max(start[index - 1] + moved, minimum), pair - minimum)
                            fractions[index] = pair - fractions[index - 1]
                            window.resize(at: path, fractions: fractions)
                        }
                        .onEnded { _ in dragStart = nil })
            }
            .zIndex(1)
    }
}

/// A pane: its conversation's transcript above its composer. Split panes
/// carry a header strip with the title; the focused one's is marked.
struct ConversationPaneView: View {
    @ObservedObject var window: ConversationWindowModel
    @ObservedObject var pane: ConversationPane
    @ObservedObject var store: AppStore
    let previewEmpty: Bool

    var body: some View {
        VStack(spacing: 0) {
            if window.isSplit { header }
            Group {
                if store.pendingConversationID != nil { ProgressView(store.conversationManagementStatus ?? "正在加载会话…") }
                else if previewEmpty || store.transcript.rows.isEmpty { emptyState }
                else { transcript }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            footer
        }
        .background(PaneAnchor { [window, id = pane.id] in window.register(id, anchor: $0) })
        .sheet(item: Binding(get: { store.pendingInteractions.first }, set: { _ in })) { RuntimeInteractionView(store: store, interaction: $0) }
        .onChange(of: store.loadedConversationID) { _, _ in pane.outlineCollapsed = false }
        .onChange(of: store.outlinePresentation) { _, _ in pane.outlineCollapsed = false }
    }

    private var focused: Bool { window.focusedPaneID == pane.id }

    private var header: some View {
        HStack(spacing: 6) {
            if let conversation = store.selectedConversationID.flatMap({ store.conversation(id: $0) }), store.isRunning(conversation) {
                ProgressView().controlSize(.mini)
            }
            Text(previewEmpty ? "新会话" : store.selectedConversationTitle ?? "新会话")
                .font(.callout.weight(focused ? .semibold : .regular))
                .foregroundStyle(focused ? .primary : .secondary)
                .lineLimit(1).truncationMode(.middle)
            Spacer(minLength: 0)
            Button { window.close(pane.id) } label: { Image(systemName: "xmark") }
                .buttonStyle(.borderless).controlSize(.small)
                .help("关闭面板").accessibilityLabel("关闭面板")
        }
        .padding(.horizontal, 10).frame(height: 26)
        .background(.bar)
        .overlay(alignment: .top) { if focused { Rectangle().fill(Color.accentColor).frame(height: 2) } }
        .overlay(alignment: .bottom) { Divider() }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(focused ? "聚焦面板" : "面板")
    }

    /// A native sub-agent is continued by the agent that started it, so its
    /// conversation has no composer.
    @ViewBuilder private var footer: some View {
        if let parent = store.loadedParentConversationID, store.pendingConversationID == nil {
            VStack(spacing: 0) {
                Divider()
                HStack(spacing: 8) {
                    Text("这是原生 sub-agent 的会话，由启动它的会话继续。").foregroundStyle(.secondary)
                    Spacer(minLength: 0)
                    Button("打开父会话") { store.openLinkedConversation(id: parent) }
                }
                .padding(.horizontal, 16).padding(.vertical, 10)
            }
            .background(.bar)
        } else {
            ComposerView(store: store, model: pane.composer, send: pane.send)
        }
    }

    private var emptyState: some View {
        ContentUnavailableView {
            Label { Text("新会话") } icon: { VeluneLogo(size: 48) }
        } description: {
            Text(store.needsModelSelection ? "选择下一轮模型后即可发送。" : store.canSend ? "输入消息，开始工作。" : "先在设置中配置AI提供商、模型与Agent 运行时。")
        } actions: {
            if !store.canSend && !store.needsModelSelection { SettingsTabLink(tab: store.enabledRuntimeInstances.isEmpty ? .runtimes : .providers, title: "打开设置") }
        }
    }

    private var transcript: some View {
        TranscriptView(model: store.transcript, conversationID: store.loadedConversationID, scrollRequest: pane.scrollRequest,
                       cwd: store.loadedConversationWorkingDirectory, remoteHost: store.loadedConversationLocation.host, outlineCollapsed: $pane.outlineCollapsed, navigation: pane.outlineNavigation,
                       activity: store.activity, loadPayloads: { [store] ids in store.loadPayloads(messageIDs: ids) },
                       openConversation: { [store] id in store.openLinkedConversation(id: id) },
                       describeSettings: { [store] in store.describe($0) })
    }
}
