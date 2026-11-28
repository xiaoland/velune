import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// A pane: its tab bar, always there as a handle to drag and a menu to
/// open, above the shown tab. Only the shown tab has views: a tab behind it
/// keeps its transcript's native list (see `TranscriptReading`), so resizing
/// the pane lays out one tab and switching back keeps the scroll position.
struct PaneView: View {
    @ObservedObject var window: ConversationWindowModel
    @ObservedObject var pane: Pane
    let previewEmpty: Bool
    @State private var size: CGSize = .zero
    @State private var dropZone: PaneDropZone?

    var body: some View {
        VStack(spacing: 0) {
            PaneTabBar(window: window, pane: pane).paneBar()
            let tab = pane.selected
            Group {
                if let tab = tab as? ConversationTab {
                    TabContentView(window: window, tab: tab, store: tab.store, previewEmpty: previewEmpty)
                } else if let tab = tab as? ArtifactTab {
                    ArtifactView(tab: tab)
                }
            }.id(tab.id)
                // Exactly the room the pane has, however wide a tab's content would like to be.
                .frame(minWidth: 0, maxWidth: .infinity, minHeight: 0, maxHeight: .infinity)
        }
        .clipped()
        .background(PaneAnchor { [window, id = pane.id] in window.register(id, anchor: $0) })
        .onGeometryChange(for: CGSize.self) { $0.size } action: { size = $0 }
        .onDrop(of: ConversationLink.dropTypes, delegate: PaneDropDelegate(window: window, pane: pane.id, size: size, zone: $dropZone))
        .overlay { if let dropZone { PaneDropHighlight(zone: dropZone) } }
    }
}

/// The conversation surface shared by ordinary editors and sheets.
struct TabContentView: View {
    @ObservedObject var window: ConversationWindowModel
    @ObservedObject var tab: ConversationTab
    @ObservedObject var store: ConversationStore
    @ObservedObject private var appearance = AppearancePreferences.shared
    let previewEmpty: Bool
    /// Over the window, where a composer the window shares does not reach.
    var inSheet = false
    @State private var forkMessageID: String?
    @State private var editingQueue: QueuedMessageEditingSession?

    var body: some View {
        GeometryReader { geometry in
        VStack(spacing: 0) {
            Group {
                if let failure = store.coldOpeningProblem, !store.retriesOpeningQuietly { openingError(failure) }
                else if store.isOpeningConversation || store.retriesOpeningQuietly { ProgressView("正在加载会话…") }
                else if previewEmpty || (store.transcript.rows.isEmpty && store.activity == nil) { emptyState }
                else { transcript }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .clipped()
            .overlay(alignment: .top) {
                TranscriptFindHost(find: tab.find, model: store.transcript, jump: tab.navigate(to:))
            }
            if let interaction = store.pendingInteractions.first {
                RuntimeInteractionView(store: store, tab: tab, interaction: interaction, maximumBodyHeight: min(220, max(64, geometry.size.height * 0.38)), contentWidth: appearance.values.composerWidth(available: max(0, geometry.size.width - 32))).id(interaction.id)
            }
            if store.hasQueuedMessages || store.followUp.isCompacting {
                SendQueueView(store: store, composer: window.composer(for: tab),
                              prepareContext: tab.showQueuedContextPreparation, editing: $editingQueue)
                    .padding(.horizontal, 16).padding(.vertical, 8)
                Divider()
            }
            footer
        }
        }
        .background(PaneAnchor { [weak window, weak tab] anchor in
            guard let tab else { return }; window?.registerConversation(tab, anchor: anchor)
        })
        .sheet(item: Binding(get: { tab.editing?.presentation == .sheet ? tab.editing : nil }, set: { value in
            if value == nil, tab.editing?.presentation == .sheet { tab.finishMessageEditing() }
        }), onDismiss: {
            // SwiftUI has now ended the native modal sheet. Ordering a panel
            // before this callback lets sheet teardown steal its key state.
            tab.showFloatingMessageEditor(in: window)
        }) { session in
            MessageEditorView(session: session, window: window, source: tab).frame(width: 560)
        }
        .onChange(of: tab.editing?.id) { _, _ in tab.showFloatingMessageEditor(in: window) }
        .sheet(item: $editingQueue) { session in
            QueuedMessageEditor(session: session, window: window, tab: tab)
        }
        .alert("创建分叉会话？", isPresented: Binding(get: { forkMessageID != nil }, set: { if !$0 { forkMessageID = nil } }), presenting: forkMessageID) { id in
            Button("取消", role: .cancel) {}
            Button("创建分叉") {
                store.forkConversation(through: id) { window.openFork(adopting: $0, device: store.device) }
            }
        } message: { _ in
            Text("新会话包含截至这一轮结束的内容。原会话和正在执行的工作会保留。")
        }
        .onChange(of: store.loadedConversationID) { _, _ in tab.outlineCollapsed = false; forkMessageID = nil }
        .onChange(of: store.device.outlinePresentation) { _, _ in tab.outlineCollapsed = false }
    }

    /// Child sessions retain their execution details while the parent owns input.
    @ViewBuilder private var footer: some View {
        if let parent = store.loadedParentConversationID, store.pendingConversationID == nil {
            VStack(spacing: 0) {
                Divider()
                HStack(spacing: 8) {
                    Text("这是原生 sub-agent 的会话，由启动它的会话继续。").foregroundStyle(.secondary)
                    Spacer(minLength: 0)
                    Button("打开父会话") { window.openLinked(parent, from: tab) }
                }
                .padding(.horizontal, 16).padding(.vertical, 10)
            }
            .background(.bar)
        }
        if inSheet || window.tools.panesHaveComposer {
            ComposerView(store: store, model: window.composer(for: tab), layout: window, send: { window.composerContext(for: tab).send() }, steer: { window.composerContext(for: tab).steer() }, submission: { window.composerContext(for: tab).submission() }, resume: { window.composerContext(for: tab).resume() })
        }
    }

    @ViewBuilder private var emptyState: some View {
        if store.isSideChat {
            ContentUnavailableView {
                Label { Text(tab.title) } icon: { VeluneLogo(size: 48) }
            } description: {
                Text("输入消息，开始工作。")
            }
        } else {
            ContentUnavailableView {
                Label { Text("新会话") } icon: { VeluneLogo(size: 48) }
            } description: {
                Text(store.canSend ? "输入消息，开始工作。" : "先在设置中配置AI提供商、模型与Agent 运行时。")
            } actions: {
                if !store.canSend { SettingsTabLink(tab: store.device.enabledRuntimeInstances.isEmpty ? .runtimes : .providers, title: "打开设置") }
            }
        }
    }

    private func openingError(_ failure: AppProblem) -> some View {
        ContentUnavailableView {
            Label(store.isSideChat ? "无法打开侧聊" : "无法打开会话", systemImage: "exclamationmark.triangle")
        } description: {
            VStack(spacing: 5) {
                Text(failure.detail)
                if let id = failure.operationID { Text("诊断编号 \(id)").font(.caption) }
            }
            .textSelection(.enabled)
        } actions: {
            if store.canRetryFailedOpening { Button("重试", action: store.retryFailedOpening) }
        }
    }

    private var transcript: some View {
        TranscriptView(model: store.transcript, reading: tab.reading, conversationID: store.loadedConversationID, scrollRequest: tab.scrollRequest,
                       cwd: store.loadedConversationWorkingDirectory, remoteHost: store.loadedConversationLocation.host, outlineCollapsed: $tab.outlineCollapsed, navigation: tab.outlineNavigation,
                       activity: store.activity, loadPayloads: { [store] ids in store.loadPayloads(messageIDs: ids) },
                       openConversation: { [window, tab] id in window.openLinked(id, from: tab) },
                       openAgentURI: window.openAgentURI, openArtifact: window.openArtifact,
                       editMessage: { [store, tab] id in store.editability(of: id) == nil ? nil : { tab.edit(messageID: id) } },
                       deleteMessage: { [store] id in store.canDeleteMessage(id) ? { store.confirmDeleteMessage(id) } : nil },
                       forkConversation: { id in if store.canFork { forkMessageID = id } },
                       describeSettings: { [store] in store.describe($0) },
                       recovery: store.runtimeRecovery, stoppingRecovery: store.isStopping,
                       canStopRecovery: store.canCancel, stopRecovery: store.cancel,
                       forking: store.isForking, deletingMessage: store.isDeletingMessage)
    }
}

/// What the bars along the top of the panes and of the docked tools share:
/// side by side under the window's toolbar they must read as one bar, so
/// the height, the rule under it and the room below the rule are set here
/// and not by each bar's content.
enum PaneChrome {
    static var barHeight: CGFloat { barHeight(for: AppearancePreferences.rendering.read()) }
    static func barHeight(for appearance: AppearanceValues) -> CGFloat { (28 * appearance.zoomScale).rounded() }
}

private struct PaneBarModifier: ViewModifier {
    @ObservedObject private var appearance = AppearancePreferences.shared
    let translucent: Bool

    func body(content: Content) -> some View {
        content.frame(height: PaneChrome.barHeight(for: appearance.values))
            .background { if !translucent { Color(nsColor: .windowBackgroundColor) } }
            .overlay(alignment: .bottom) { TabShade.line.frame(height: 1) }
            .padding(.bottom, 1)
    }
}

extension View {
    func paneBar(translucent: Bool = false) -> some View {
        modifier(PaneBarModifier(translucent: translucent))
    }
}

/// The system tab bar's shades over the window's colour: tabs not shown are
/// darker with a soft shadow under the bar's top edge, and lines part them.
enum TabShade {
    static let line = Color(nsColor: .separatorColor)

    static var unselected: some View {
        Color.black.opacity(0.065)
            .overlay(alignment: .top) {
                LinearGradient(colors: [Color.black.opacity(0.12), .clear], startPoint: .top, endPoint: .bottom).frame(height: 3)
            }
    }
}

/// A menu item that runs a closure.
final class ClosureMenuItem: NSMenuItem {
    private let run: () -> Void

    init(_ title: String, run: @escaping () -> Void) {
        self.run = run
        super.init(title: title, action: #selector(runClosure), keyEquivalent: "")
        target = self
    }

    required init(coder: NSCoder) { fatalError("not decoded") }

    @objc private func runClosure() { run() }
}

/// Where a tab dragged over a pane would go: the nearest edge within a
/// quarter of the pane splits there; the middle adds it to the pane's tabs.
private struct PaneDropDelegate: DropDelegate {
    let window: ConversationWindowModel
    let pane: UUID
    let size: CGSize
    @Binding var zone: PaneDropZone?

    func validateDrop(info: DropInfo) -> Bool { info.hasItemsConforming(to: ConversationLink.dropTypes) }

    func dropUpdated(info: DropInfo) -> DropProposal? {
        let x = info.location.x / max(size.width, 1), y = info.location.y / max(size.height, 1)
        let nearest = [(PaneEdge.leading, x), (.trailing, 1 - x), (.top, y), (.bottom, 1 - y)].min { $0.1 < $1.1 }!
        let zone: PaneDropZone = nearest.1 < 0.25 ? .edge(nearest.0) : .center
        if self.zone != zone { self.zone = zone }
        return DropProposal(operation: .move)
    }

    func dropExited(info: DropInfo) { zone = nil }

    func performDrop(info: DropInfo) -> Bool {
        guard let zone, let provider = info.itemProviders(for: ConversationLink.dropTypes).first else { zone = nil; return false }
        let source = ListDragWatch.shared.sourceWindow
        ListDragWatch.shared.accepted()
        self.zone = nil
        ConversationLink.load(provider) { [window, pane] url in
            Task { @MainActor in window.drop(url, on: pane, zone: zone, from: source) }
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
