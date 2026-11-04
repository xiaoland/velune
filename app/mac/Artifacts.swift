import AppKit
import CryptoKit
import SwiftUI
import WebKit

/// A source snapshot, independent of where it is displayed. Presentations
/// carry this identity through sheets, editor groups and window handoffs.
struct ArtifactDocument: Codable, Equatable, Identifiable {
    enum Kind: String, Codable { case mermaid }
    let id: String
    let kind: Kind
    let source: String
    let conversationID: String?
    let sourceID: String
    var title: String { switch kind { case .mermaid: "Mermaid 图表" } }

    static func mermaid(source: String, conversationID: String?, sourceID: String) -> Self {
        let digest = SHA256.hash(data: Data(source.utf8)).map { String(format: "%02x", $0) }.joined()
        return Self(id: "\(conversationID ?? ""):\(sourceID):\(digest)", kind: .mermaid, source: source, conversationID: conversationID, sourceID: sourceID)
    }
}

@MainActor final class ArtifactTab: EditorTab {
    let document: ArtifactDocument
    @Published var showsSource = false
    init(id: UUID = UUID(), document: ArtifactDocument) { self.document = document; super.init(id: id) }
    override var title: String { document.title }
}

struct ArtifactView: View {
    @ObservedObject var tab: ArtifactTab
    private var document: ArtifactDocument { tab.document }
    @State private var rendererError: String?
    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Picker("显示", selection: $tab.showsSource) {
                    Text("图表").tag(false)
                    Text("源代码").tag(true)
                }.pickerStyle(.segmented).fixedSize()
                Spacer()
                Button("复制源代码") {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(document.source, forType: .string)
                }.buttonStyle(.borderless)
            }.padding(10)
            Divider()
            if tab.showsSource {
                GeometryReader { geometry in
                    ScrollView([.horizontal, .vertical]) {
                        Text(document.source).font(.system(.body, design: .monospaced)).textSelection(.enabled)
                            .frame(minWidth: max(0, geometry.size.width - 32), minHeight: max(0, geometry.size.height - 32), alignment: .topLeading)
                            .padding(16)
                    }
                }.background(Color(nsColor: .textBackgroundColor))
            } else {
                switch document.kind {
                case .mermaid:
                    MermaidArtifactView(source: document.source, failure: $rendererError)
                        .overlay {
                            if let rendererError {
                                ContentUnavailableView("无法显示图表", systemImage: "exclamationmark.triangle", description: Text(rendererError))
                            }
                        }
                }
            }
        }.frame(minWidth: 0, maxWidth: .infinity, minHeight: 0, maxHeight: .infinity)
    }
}

struct ArtifactSheet: View {
    @ObservedObject var window: ConversationWindowModel
    let tab: ArtifactTab
    var body: some View {
        VStack(spacing: 0) {
            HStack {
                ArtifactTabItem(window: window, tab: tab, selected: true, place: 0, count: 1, slide: { _ in })
                    .frame(width: 200, height: PaneChrome.barHeight)
                    .help("拖到标签栏、分屏或窗口外")
                Spacer()
                Button("移到标签页") { window.moveArtifactSheetToTab() }
                Button("移到新窗口") { window.moveArtifactSheetToWindow() }
                Button("关闭") { window.closeArtifactSheet() }.keyboardShortcut(.cancelAction)
            }.padding(10)
            Divider()
            ArtifactView(tab: tab)
        }.frame(minWidth: 520, idealWidth: 820, minHeight: 360, idealHeight: 600)
    }
}

/// Reuses the editor drag handle; artifacts never create a conversation
/// store, start a runtime or participate in composer lifecycle.
struct ArtifactTabItem: View {
    @ObservedObject var window: ConversationWindowModel
    let tab: ArtifactTab
    let selected: Bool
    let place: Int
    let count: Int
    let slide: (TabSlide) -> Void
    @State private var hovering = false
    var body: some View {
        ZStack {
            TabHandle(window: window, tab: tab, title: tab.title, place: place, count: count, actions: .init(
                select: { window.selectTab(tab.id) }, rename: {}, close: { close() },
                hover: { hovering = $0 }, closePointer: { _ in }, slide: slide, menu: menu))
            HStack(spacing: 5) {
                Image(systemName: hovering ? "xmark" : "flowchart").frame(width: 16)
                Text(tab.title).lineLimit(1).truncationMode(.tail)
                Spacer(minLength: 0)
            }.font(.system(size: 11)).foregroundStyle(selected ? .primary : .secondary)
                .padding(.horizontal, 8).allowsHitTesting(false)
        }
        .background(Color(nsColor: .windowBackgroundColor))
        .overlay(alignment: .trailing) { TabShade.line.frame(width: 1) }
    }
    private func close() {
        if window.artifactSheet?.id == tab.id { window.closeArtifactSheet() } else { window.closeTab(tab.id) }
    }
    private func menu() -> NSMenu {
        NSHostingMenu(rootView: EditorTabActions(window: window, tabID: tab.id))
    }
}

/// Local-only renderer. Diagram text crosses the JS boundary as a value,
/// never as HTML. Navigation, network and Mermaid links are disabled.
private struct MermaidArtifactView: NSViewRepresentable {
    let source: String
    @Binding var failure: String?
    func makeCoordinator() -> Coordinator { Coordinator(source: source, onError: { failure = $0 }) }
    func makeNSView(context: Context) -> WKWebView {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .nonPersistent()
        let view = WKWebView(frame: .zero, configuration: configuration)
        view.navigationDelegate = context.coordinator
        view.allowsMagnification = true
        if let page = Bundle.main.url(forResource: "viewer", withExtension: "html", subdirectory: "Artifacts") {
            view.loadFileURL(page, allowingReadAccessTo: page.deletingLastPathComponent())
        } else {
            DispatchQueue.main.async { failure = "找不到随应用安装的图表渲染器。" }
        }
        return view
    }
    func updateNSView(_ view: WKWebView, context: Context) {}
    @MainActor final class Coordinator: NSObject, WKNavigationDelegate {
        let source: String
        let onError: (String?) -> Void
        init(source: String, onError: @escaping (String?) -> Void) { self.source = source; self.onError = onError }
        func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
            webView.callAsyncJavaScript("await renderArtifact(source)", arguments: ["source": source], in: nil, in: .page, completionHandler: { [onError] result in
                switch result {
                case .failure(let error): onError(error.localizedDescription)
                case .success: onError(nil)
                }
            })
        }
        func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) { onError(error.localizedDescription) }
        func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) { onError(error.localizedDescription) }
        func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction, decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {
            decisionHandler(action.navigationType == .other && action.request.url?.isFileURL == true ? .allow : .cancel)
        }
    }
}
