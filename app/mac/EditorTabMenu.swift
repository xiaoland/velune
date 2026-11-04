import SwiftUI

/// Layout actions shared by every editor document, independent of its kind.
struct EditorTabActions: View {
    @ObservedObject var window: ConversationWindowModel
    let tabID: UUID
    private var inSheet: Bool { window.artifactSheet?.id == tabID }
    private var siblings: Int { window.pane(holding: tabID)?.tabs.count ?? 0 }
    var body: some View {
        Button("关闭标签页") { if inSheet { window.closeArtifactSheet() } else { window.closeTab(tabID) } }
        if inSheet {
            Button("移到标签页") { window.moveArtifactSheetToTab() }
            Button("移到新窗口") { window.moveArtifactSheetToWindow() }
        } else {
            if siblings > 1 { Button("关闭其他标签页") { window.closeOtherTabs(tabID) } }
            Divider()
            if siblings > 1 {
                Button("移到右侧拆分") { window.splitTab(tabID, edge: .trailing) }
                Button("移到下方拆分") { window.splitTab(tabID, edge: .bottom) }
            } else {
                Button("向右拆分") { if let pane = window.pane(holding: tabID) { window.focus(pane.id) }; window.split(.horizontal) }
                Button("向下拆分") { if let pane = window.pane(holding: tabID) { window.focus(pane.id) }; window.split(.vertical) }
            }
            if window.canMoveTabToNewWindow { Button("移到新窗口") { window.moveToNewWindow(tabID) } }
        }
        Divider()
        Menu("布局") {
            ForEach(ToolWindowKind.allCases, id: \.self) { kind in
                Menu("\(kind.title)位置") { ToolPlaceMenu(window: window, kind: kind) }
            }
            Divider()
            Button(window.tools.tools(in: .bottom).isEmpty ? "显示底部区域" : "隐藏底部区域") { window.tools.toggleArea(.bottom) }
            Button(window.tools.isEditorOnly ? "恢复停靠区域" : "只显示编辑区") { window.tools.toggleEditorOnly() }
            Toggle("保持在最前面", isOn: $window.pinned)
        }
    }
}
