import AppKit
import SwiftUI
import UniformTypeIdentifiers

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
    private static let minimumLength: CGFloat = 320

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
    @State private var size: CGSize = .zero
    @State private var dropZone: PaneDropZone?

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
        .onGeometryChange(for: CGSize.self) { $0.size } action: { size = $0 }
        .onDrop(of: [.url], delegate: PaneDropDelegate(window: window, pane: pane.id, size: size, zone: $dropZone))
        .overlay { if let dropZone { PaneDropHighlight(zone: dropZone) } }
        .sheet(item: Binding(get: { store.pendingInteractions.first }, set: { _ in })) { RuntimeInteractionView(store: store, interaction: $0) }
        .onChange(of: store.loadedConversationID) { _, _ in pane.outlineCollapsed = false }
        .onChange(of: store.outlinePresentation) { _, _ in pane.outlineCollapsed = false }
    }

    private var focused: Bool { window.focusedPaneID == pane.id }

    private var header: some View {
        HStack(spacing: 6) {
            HStack(spacing: 6) {
                if let conversation = store.selectedConversationID.flatMap({ store.conversation(id: $0) }), store.isRunning(conversation) {
                    ProgressView().controlSize(.mini)
                }
                Text(previewEmpty ? "新会话" : store.selectedConversationTitle ?? "新会话")
                    .font(.callout.weight(focused ? .semibold : .regular))
                    .foregroundStyle(focused ? .primary : .secondary)
                    .lineLimit(1).truncationMode(.middle)
                Spacer(minLength: 0)
            }
            .frame(maxHeight: .infinity)
            // The title strip carries the pane: to another pane, another window, or out into a new one.
            .overlay { PaneDragHandle(window: window, pane: pane) }
            .help(pane.link == nil ? "" : "拖动以移到其他位置或其他窗口；拖到窗口外则在新窗口中打开")
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
        } else if window.tools.composer == .docked {
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

/// Where a conversation dragged over a pane would go: the nearest edge
/// within a quarter of the pane splits there; the middle shows it here.
private struct PaneDropDelegate: DropDelegate {
    let window: ConversationWindowModel
    let pane: UUID
    let size: CGSize
    @Binding var zone: PaneDropZone?

    func validateDrop(info: DropInfo) -> Bool { info.hasItemsConforming(to: [.url]) }

    func dropUpdated(info: DropInfo) -> DropProposal? {
        let x = info.location.x / max(size.width, 1), y = info.location.y / max(size.height, 1)
        let nearest = [(PaneEdge.leading, x), (.trailing, 1 - x), (.top, y), (.bottom, 1 - y)].min { $0.1 < $1.1 }!
        let zone: PaneDropZone = nearest.1 < 0.25 ? .edge(nearest.0) : .center
        if self.zone != zone { self.zone = zone }
        return DropProposal(operation: .move)
    }

    func dropExited(info: DropInfo) { zone = nil }

    func performDrop(info: DropInfo) -> Bool {
        guard let zone, let provider = info.itemProviders(for: [.url]).first else { zone = nil; return false }
        self.zone = nil
        _ = provider.loadObject(ofClass: NSURL.self) { [window, pane] object, _ in
            guard let url = object as? URL, let link = ConversationLink.conversation(in: url) else { return }
            Task { @MainActor in window.drop(link.conversationID, on: pane, zone: zone) }
        }
        return true
    }
}

/// The part of a pane a drop would take.
private struct PaneDropHighlight: View {
    let zone: PaneDropZone

    var body: some View {
        GeometryReader { geometry in
            let size = geometry.size
            let rect: CGRect = switch zone {
            case .center: CGRect(origin: .zero, size: size)
            case .edge(.leading): CGRect(x: 0, y: 0, width: size.width / 2, height: size.height)
            case .edge(.trailing): CGRect(x: size.width / 2, y: 0, width: size.width / 2, height: size.height)
            case .edge(.top): CGRect(x: 0, y: 0, width: size.width, height: size.height / 2)
            case .edge(.bottom): CGRect(x: 0, y: size.height / 2, width: size.width, height: size.height / 2)
            }
            RoundedRectangle(cornerRadius: 6)
                .fill(Color.accentColor.opacity(0.12))
                .strokeBorder(Color.accentColor.opacity(0.6), lineWidth: 2)
                .frame(width: max(0, rect.width - 8), height: max(0, rect.height - 8))
                .position(x: rect.midX, y: rect.midY)
        }
        .allowsHitTesting(false)
        .animation(.easeOut(duration: 0.12), value: zone)
    }
}

/// A pane's title strip as an AppKit drag source, since only AppKit tells
/// where a drag ended: dropped where nothing takes it, outside every
/// window, the pane becomes a window of its own.
private struct PaneDragHandle: NSViewRepresentable {
    let window: ConversationWindowModel
    let pane: ConversationPane

    func makeNSView(context: Context) -> HandleView { HandleView() }

    func updateNSView(_ view: HandleView, context: Context) {
        view.model = window
        view.pane = pane
    }

    final class HandleView: NSView, NSDraggingSource {
        weak var model: ConversationWindowModel?
        weak var pane: ConversationPane?
        private var pressed: NSEvent?

        override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
        override func mouseDown(with event: NSEvent) { pressed = event }
        override func mouseUp(with event: NSEvent) { pressed = nil }

        override func mouseDragged(with event: NSEvent) {
            guard let start = pressed, let pane, let link = pane.link, let conversationID = pane.store.loadedConversationID else { return }
            let from = start.locationInWindow, to = event.locationInWindow
            guard hypot(to.x - from.x, to.y - from.y) > 4 else { return }
            pressed = nil
            PaneDragSource.shared.begin(pane: pane.id, conversationID: conversationID)
            let board = NSPasteboardItem()
            board.setString(link.absoluteString, forType: .URL)
            board.setString(link.absoluteString, forType: .string)
            let item = NSDraggingItem(pasteboardWriter: board)
            let image = snapshot(of: pane.id)
            let point = convert(from, from: nil)
            item.setDraggingFrame(NSRect(x: point.x - image.size.width / 2, y: point.y - image.size.height + 12, width: image.size.width, height: image.size.height), contents: image)
            beginDraggingSession(with: [item], event: start, source: self)
        }

        func draggingSession(_ session: NSDraggingSession, sourceOperationMaskFor context: NSDraggingContext) -> NSDragOperation {
            context == .withinApplication ? .move : .copy
        }

        func draggingSession(_ session: NSDraggingSession, endedAt screenPoint: NSPoint, operation: NSDragOperation) {
            guard operation == [], let model, let pane,
                  !NSApp.windows.contains(where: { $0.isVisible && $0.frame.contains(screenPoint) }) else { return }
            model.tearOff(pane.id, at: screenPoint)
        }

        /// The pane, reduced, to drag around.
        private func snapshot(of id: UUID) -> NSImage {
            guard let view = model?.view(of: id), let content = view.window?.contentView else { return NSImage(size: NSSize(width: 1, height: 1)) }
            let rect = view.convert(view.bounds, to: content)
            guard rect.width > 0, rect.height > 0, let representation = content.bitmapImageRepForCachingDisplay(in: rect) else { return NSImage(size: NSSize(width: 1, height: 1)) }
            content.cacheDisplay(in: rect, to: representation)
            let scale = min(1, 320 / rect.width)
            let image = NSImage(size: NSSize(width: rect.width * scale, height: rect.height * scale))
            image.lockFocus()
            representation.draw(in: NSRect(origin: .zero, size: image.size))
            image.unlockFocus()
            return image
        }
    }
}
