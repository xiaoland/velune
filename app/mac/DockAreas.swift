import AppKit
import SwiftUI

/// A dock area's tools: stacked down the sidebar or the inspector, side by
/// side below the panes. A composer in a side area keeps its own height
/// under the others. With more than one tool each has a title strip.
struct DockAreaView: View {
    @ObservedObject var window: ConversationWindowModel
    let side: DockSide

    var body: some View {
        let kinds = window.tools.tools(in: side)
        let flexible = side == .bottom ? kinds : kinds.filter { $0 != .composer }
        VStack(spacing: 0) {
            if flexible.count == 1, let kind = flexible.first {
                tool(kind, titled: kinds.count > 1)
            } else if !flexible.isEmpty {
                WeightedSplit(axis: side == .bottom ? .horizontal : .vertical, ids: flexible,
                              weights: flexible.map { window.tools[$0].weight },
                              commit: { weights in window.tools.setWeights(Dictionary(uniqueKeysWithValues: zip(flexible, weights))) }) { kind in
                    tool(kind, titled: kinds.count > 1)
                }
            }
            if side != .bottom, kinds.contains(.composer) {
                if !flexible.isEmpty { Divider() }
                tool(.composer, titled: kinds.count > 1)
            }
        }
    }

    private func tool(_ kind: ToolWindowKind, titled: Bool) -> some View {
        VStack(spacing: 0) {
            if titled { ToolHeader(window: window, kind: kind) }
            ToolContent(kind: kind, window: window, side: side)
                // A composer under other tools keeps its own height; elsewhere it sits at the bottom of its room.
                .frame(maxWidth: .infinity, maxHeight: kind == .composer && side != .bottom && window.tools.tools(in: side).count > 1 ? nil : .infinity,
                       alignment: .bottom)
        }
    }
}

/// A docked tool's title, with its places and a button that hides it.
private struct ToolHeader: View {
    @ObservedObject var window: ConversationWindowModel
    let kind: ToolWindowKind

    var body: some View {
        HStack(spacing: 4) {
            Text(kind.title).font(.subheadline.weight(.semibold)).foregroundStyle(.secondary).lineLimit(1)
            Spacer(minLength: 0)
            Menu { ToolPlaceMenu(window: window, kind: kind) } label: { Image(systemName: "ellipsis") }
                .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize()
                .help("移到其他位置").accessibilityLabel("\(kind.title)的位置")
            Button { window.tools.hide(kind) } label: { Image(systemName: "xmark") }
                .buttonStyle(.borderless).controlSize(.small)
                .help(kind == .composer ? "放回各个面板" : "隐藏\(kind.title)").accessibilityLabel("隐藏\(kind.title)")
        }
        .padding(.leading, 12).padding(.trailing, 8).frame(height: 26)
        .contentShape(Rectangle())
        .contextMenu { ToolPlaceMenu(window: window, kind: kind) }
        .overlay(alignment: .bottom) { Divider() }
    }
}

/// A tool's places, as a picker in a menu.
struct ToolPlaceMenu: View {
    @ObservedObject var window: ConversationWindowModel
    let kind: ToolWindowKind

    var body: some View {
        Picker("位置", selection: Binding(get: { window.choice(for: kind) }, set: { window.choose($0, for: kind) })) {
            ForEach(ToolChoice.all(for: kind), id: \.choice) { Text($0.title).tag($0.choice) }
        }
        .pickerStyle(.inline).labelsHidden()
    }
}

/// Tools side by side or stacked, each taking its share of the room, with
/// dividers between them. A divider's drag is the view's until it ends; a
/// double click shares the room equally again.
struct WeightedSplit<ID: Hashable, Content: View>: View {
    let axis: Axis
    let ids: [ID]
    let weights: [Double]
    let commit: ([Double]) -> Void
    @ViewBuilder let content: (ID) -> Content
    @State private var dragged: [Double]?
    @State private var dragStart: [Double]?
    private static var minimumLength: CGFloat { 120 }

    var body: some View {
        GeometryReader { geometry in
            let horizontal = axis == .horizontal
            let length = (horizontal ? geometry.size.width : geometry.size.height) - CGFloat(ids.count - 1)
            let fractions = normalized(dragged ?? weights)
            let stack = horizontal ? AnyLayout(HStackLayout(spacing: 0)) : AnyLayout(VStackLayout(spacing: 0))
            stack {
                ForEach(Array(ids.enumerated()), id: \.element) { index, id in
                    if index > 0 { Divider() }
                    content(id)
                        .frame(width: horizontal ? max(0, length * fractions[index]) : geometry.size.width,
                               height: horizontal ? geometry.size.height : max(0, length * fractions[index]))
                        .clipped()
                }
            }
            .overlay {
                SashOverlay(axis: axis, lengths: fractions.map { max(0, length * $0) },
                            drag: { index, distance in drag(before: index, distance: distance, length: length, fractions: fractions) },
                            end: {
                                if let dragged { commit(dragged) }
                                dragged = nil
                                dragStart = nil
                            },
                            reset: { commit(ids.map { _ in 1 }) })
            }
        }
    }

    private func normalized(_ values: [Double]) -> [Double] {
        guard values.count == ids.count else { return ids.map { _ in 1 / Double(max(ids.count, 1)) } }
        let total = values.reduce(0, +)
        return total > 0 ? values.map { $0 / total } : values.map { _ in 1 / Double(values.count) }
    }

    private func drag(before index: Int, distance: CGFloat, length: CGFloat, fractions: [Double]) {
        let start = dragStart ?? fractions
        if dragStart == nil { dragStart = start }
        guard length > 0 else { return }
        let pair = start[index - 1] + start[index]
        let minimum = min(pair / 2, Double(Self.minimumLength / length))
        var next = start
        next[index - 1] = min(max(start[index - 1] + Double(distance / length), minimum), pair - minimum)
        next[index] = pair - next[index - 1]
        dragged = next
    }
}

/// The strip below the panes: as tall as the user made it, or, holding the
/// composer alone, as tall as the composer. Its top divider's grip reaches
/// over the panes' edge; a double click restores the usual height.
struct BottomAreaView: View {
    @ObservedObject var window: ConversationWindowModel
    /// The room the panes and this area share.
    let available: CGFloat
    @State private var dragged: Double?
    @State private var dragStart: Double?

    var body: some View {
        let kinds = window.tools.tools(in: .bottom)
        let fits = kinds == [.composer]
        let limit = max(100, available - 200)
        VStack(spacing: 0) {
            Divider()
            DockAreaView(window: window, side: .bottom)
                .frame(height: fits ? nil : min(dragged ?? window.tools.bottomHeight, limit))
        }
        .overlay(alignment: .top) {
            if !fits {
                SplitSash(axis: .vertical,
                          drag: { distance in
                              let start = dragStart ?? min(window.tools.bottomHeight, limit)
                              if dragStart == nil { dragStart = start }
                              dragged = min(max(start - distance, 100), limit)
                          },
                          end: {
                              if let dragged { window.tools.bottomHeight = dragged }
                              dragged = nil
                              dragStart = nil
                          },
                          reset: { window.tools.bottomHeight = ToolWindowLayout().bottomHeight })
                    .frame(height: SplitSash.thickness)
                    .offset(y: -SplitSash.thickness / 2)
            }
        }
    }
}

/// A tool's content, wherever it is, following the window's focused pane.
struct ToolContent: View {
    let kind: ToolWindowKind
    @ObservedObject var window: ConversationWindowModel
    /// The dock area it is in; nil outside the window.
    var side: DockSide?
    /// For a window sized to it.
    var contentHeight: ((CGFloat) -> Void)?

    var body: some View {
        let tab = window.focused
        switch kind {
        case .list:
            // The sidebar's own search field and toolbar serve the list at its top.
            ConversationListView(window: window, store: window.list, focused: tab.store,
                                 placement: side == .left && window.tools.tools(in: .left).first == .list ? .sidebar : .panel)
        case .outline:
            ToolOutline(tab: tab, store: tab.store, inSidebar: side == .left).id(tab.id)
        case .composer:
            ToolComposer(window: window, tab: tab, store: tab.store, contentHeight: contentHeight).id(tab.id)
        }
    }
}

/// A floating panel's or tool window's content, which names the window's
/// conversation in a tool window's subtitle.
struct DetachedToolContent: View {
    let kind: ToolWindowKind
    @ObservedObject var window: ConversationWindowModel

    var body: some View {
        ToolContent(kind: kind, window: window, contentHeight: kind == .composer ? { window.detached.fitComposer(contentHeight: $0) } : nil)
            .modifier(SubtitleFollower(window: window, store: window.focused.store, kind: kind))
    }
}

private struct SubtitleFollower: ViewModifier {
    let window: ConversationWindowModel
    @ObservedObject var store: AppStore
    let kind: ToolWindowKind

    func body(content: Content) -> some View {
        content.onChange(of: store.selectedConversationTitle ?? "新会话", initial: true) { _, title in
            window.detached.setSubtitle(title, for: kind)
        }
    }
}

/// The focused tab's outline, docked, floating or in a window.
struct ToolOutline: View {
    @ObservedObject var tab: ConversationTab
    @ObservedObject var store: AppStore
    let inSidebar: Bool

    var body: some View {
        if store.pendingConversationID != nil || store.transcript.userRows.isEmpty {
            ContentUnavailableView("没有用户消息", systemImage: "list.bullet", description: Text("会话有了用户消息后，在这里列出并可跳转。"))
        } else {
            TranscriptOutlineView(model: store.transcript, inPopover: false, inSidebar: inSidebar,
                                  editMessage: { [store, tab] id in store.editability(of: id) == nil ? nil : { tab.edit(messageID: id) } }) { tab.navigate(to: $0) }
        }
    }
}

/// One composer for the window, sending to the focused tab.
private struct ToolComposer: View {
    @ObservedObject var window: ConversationWindowModel
    @ObservedObject var tab: ConversationTab
    @ObservedObject var store: AppStore
    let contentHeight: ((CGFloat) -> Void)?

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if window.isSplit || window.tabCount > 1 {
                Text("发往：\(store.selectedConversationTitle ?? "新会话")")
                    .font(.caption).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
                    .padding(.horizontal, 16).padding(.top, 8)
            }
            if store.loadedParentConversationID != nil, store.pendingConversationID == nil {
                Text("这是原生 sub-agent 的会话，由启动它的会话继续。").foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity).padding(16)
            } else {
                ComposerView(store: store, model: tab.composer, send: tab.send)
            }
        }
        .fixedSize(horizontal: false, vertical: true)
        .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { contentHeight?($0) }
        .frame(maxHeight: contentHeight == nil ? nil : .infinity, alignment: .top)
    }
}
