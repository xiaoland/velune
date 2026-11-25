import AppKit
import SwiftUI

/// A dock area's tools: stacked down the sidebar or the inspector, side by
/// side below the panes, in a native split when there are several. A
/// composer in a side area keeps its own height under the others. With more
/// than one tool each has a title strip.
struct DockAreaView: View {
    @ObservedObject var window: ConversationWindowModel
    let side: DockSide

    var body: some View {
        let kinds = window.tools.kept(in: side)
        let flexible = side == .bottom ? kinds : kinds.filter { $0 != .composer }
        VStack(spacing: 0) {
            // A tool alone is the area's content itself.
            if flexible.count == 1, let kind = flexible.first {
                DockedTool(window: window, kind: kind, side: side)
            } else if flexible.count > 1 {
                ToolSplit(window: window, side: side, kinds: flexible)
            }
            if side != .bottom, kinds.contains(.composer) {
                if !flexible.isEmpty { Divider() }
                DockedTool(window: window, kind: .composer, side: side)
            }
        }
    }
}

/// One docked tool, with its title strip when it shares its area.
struct DockedTool: View {
    @ObservedObject var window: ConversationWindowModel
    let kind: ToolWindowKind
    let side: DockSide

    var body: some View {
        let shared = window.tools.kept(in: side).count > 1
        VStack(spacing: 0) {
            if shared { ToolHeader(window: window, kind: kind, side: side) }
            ToolContent(kind: kind, window: window, side: side)
                // A composer under other tools keeps its own height; elsewhere it sits at the bottom of its room.
                .frame(maxWidth: .infinity, maxHeight: kind == .composer && side != .bottom && shared ? nil : .infinity,
                       alignment: .bottom)
        }
    }
}

/// A docked tool's title, with its places and a button that hides it.
private struct ToolHeader: View {
    @ObservedObject var window: ConversationWindowModel
    let kind: ToolWindowKind
    let side: DockSide

    var body: some View {
        HStack(spacing: 4) {
            Text(kind.title).uiFont(.subheadline.weight(.semibold)).foregroundStyle(.secondary).lineLimit(1)
            Spacer(minLength: 0)
            Menu { ToolPlaceMenu(window: window, kind: kind) } label: { Image(systemName: "ellipsis") }
                .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize()
                .help("移到其他位置").accessibilityLabel("\(kind.title)的位置")
            Button { window.tools.hide(kind) } label: { Image(systemName: "xmark") }
                .buttonStyle(.borderless).controlSize(.small)
                .help(kind == .composer ? "放回各个面板" : "隐藏\(kind.title)").accessibilityLabel("隐藏\(kind.title)")
        }
        .padding(.leading, 12).padding(.trailing, 8)
        .paneBar(translucent: true)
        .contentShape(Rectangle())
        .contextMenu { ToolPlaceMenu(window: window, kind: kind) }
    }
}

/// A tool's places under its name, for the context menu of the tool itself.
struct ToolPlaceSection: View {
    @ObservedObject var window: ConversationWindowModel
    let kind: ToolWindowKind

    var body: some View {
        Section("\(kind.title)位置") { ToolPlaceMenu(window: window, kind: kind) }
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
            ConversationListView(window: window, store: window.list, focused: tab.store)
        case .outline:
            ToolOutline(window: window, tab: tab, store: tab.store, inSidebar: side == .left).id(tab.id)
        case .composer:
            if !window.hasConversationFocus {
                Text("选择会话以输入").foregroundStyle(.secondary).frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
            ToolComposer(window: window, tab: tab, store: tab.store, contentHeight: contentHeight)
            }
        case .agents:
            ToolAgents(window: window, tab: tab, store: tab.store, inSidebar: side == .left).id(tab.id)
        case .backgroundTasks:
            BackgroundTasksView(store: tab.store, inPopover: false, inSidebar: side == .left, layout: window) {
                window.openLinked($0, from: tab, opening: $1)
            }.id(tab.id)
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
    @ObservedObject var store: ConversationStore
    let kind: ToolWindowKind

    func body(content: Content) -> some View {
        content.onChange(of: store.selectedConversationTitle ?? "新会话", initial: true) { _, title in
            window.detached.setSubtitle(title, for: kind)
        }
    }
}

/// The focused tab's outline, docked, floating or in a window.
struct ToolOutline: View {
    let window: ConversationWindowModel
    @ObservedObject var tab: ConversationTab
    @ObservedObject var store: ConversationStore
    let inSidebar: Bool

    var body: some View {
        if store.pendingConversationID != nil || store.transcript.userRows.isEmpty {
            ContentUnavailableView("没有用户消息", systemImage: "list.bullet", description: Text("会话有了用户消息后，在这里列出并可跳转。"))
        } else {
            TranscriptOutlineView(model: store.transcript, inPopover: false, inSidebar: inSidebar, layout: window,
                                  editMessage: { [store, tab] id in store.editability(of: id) == nil ? nil : { tab.edit(messageID: id) } },
                                  deleteMessage: { [store] id in store.canDeleteMessage(id) ? { store.confirmDeleteMessage(id) } : nil },
                                  isRunning: store.loadedConversationID.map(store.device.isConversationRunning) ?? false,
                                  goToBottom: { [tab] in tab.scrollRequest &+= 1 }) { tab.navigate(to: $0) }
        }
    }
}

private struct ToolAgents: View {
    let window: ConversationWindowModel
    let tab: ConversationTab
    @ObservedObject var store: ConversationStore
    let inSidebar: Bool

    var body: some View {
        LinkedAgentsView(store: store, inPopover: false, inSidebar: inSidebar, layout: window) { window.openLinked($0, from: tab, opening: $1) }
    }
}

/// One composer for the window, sending to the focused tab.
private struct ToolComposer: View {
    @ObservedObject var window: ConversationWindowModel
    @ObservedObject var tab: ConversationTab
    @ObservedObject var store: ConversationStore
    let contentHeight: ((CGFloat) -> Void)?

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if store.loadedParentConversationID != nil, store.pendingConversationID == nil {
                Text("这是原生 sub-agent 的会话，由启动它的会话继续。").foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity).padding(16)
            }
            ComposerView(store: store, model: window.composer(for: tab), layout: window, send: { window.send() }, steer: { window.composerContext().steer() }, submission: { window.composerContext().submission() }, resume: { window.resume() },
                             fillsHeight: window.tools[.composer].place == .docked(.bottom),
                             recipient: window.composerOwner(for: tab) == .shared ? store.selectedConversationTitle ?? "新会话" : nil)
        }
        .fixedSize(horizontal: false, vertical: window.tools[.composer].place != .docked(.bottom))
        .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { contentHeight?($0) }
        .frame(maxHeight: contentHeight != nil || window.tools[.composer].place == .docked(.bottom) ? .infinity : nil, alignment: .top)
    }
}
