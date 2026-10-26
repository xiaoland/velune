import SwiftUI

/// Every command's shortcut as it is now, then the keys each place keeps
/// for itself while it has the keyboard.
struct ShortcutHelpView: View {
    @ObservedObject var window: ConversationWindowModel
    @ObservedObject private var shortcuts = KeyboardShortcutSettings.shared
    @Environment(\.dismiss) private var dismiss

    private var placeKeys: [(String, [(String, String)])] {
        let mode = shortcuts.sendMode
        return [
            ("输入区", [
                (mode.sendKey, "发送；运行中排队"),
                (mode.steerKey, "运行中插入本轮（运行时不支持时排队）"),
                (mode == .returnSends ? "⇧Return、⌥Return" : "Return", "换行"),
                ("⌥↑", "取回最后一条排队消息；输入框有内容时编辑它"),
                ("/", "Velune 命令"),
                ("@", "插入工作目录中的文件路径"),
            ]),
            ("命令面板与打开会话", [("↑ ↓", "选择"), ("Return", "执行或打开"), ("⌘Return", "在新标签页打开会话"), ("⎋", "关闭")]),
            ("审批与提问卡片", [("1–9", "选择对应的选项"), ("⌘⌫", "取消请求"), ("Return", "回答完毕后提交提问"), ("Tab", "在提问的字段之间移动")]),
            ("消息列表", [
                ("↑ ↓", "上一条、下一条"), ("⌥↑ ⌥↓", "上一条、下一条用户消息"), ("⌘↑ ⌘↓", "第一条、最后一条"),
                ("Return、Space", "展开或收起工作过程"), ("⌘C", "拷贝所选消息"), ("⌥⌘C", "拷贝其中最后一个代码块"), ("E", "编辑所选的用户消息"),
            ]),
            ("会话列表", [("↑ ↓", "选择并打开"), ("Return", "重命名"), ("⌘⌫", "删除"), ("输入文字", "转到搜索")]),
            ("Esc", [("⎋", "依次：结束输入法组词、关闭弹出列表、关闭查找栏、关闭表单，最后停止生成")]),
        ]
    }

    var body: some View {
        VStack(spacing: 0) {
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    ForEach(ShortcutCommand.Menu.allCases) { menu in
                        let commands = ShortcutCommand.allCases.filter { $0.menu == menu && shortcuts.shortcut($0) != nil }
                        if !commands.isEmpty { section(menu.sectionTitle, commands.map { (shortcuts.shortcut($0)!.title, $0.title) }) }
                    }
                    ForEach(placeKeys, id: \.0) { section($0.0, $0.1) }
                }
                .padding(20)
            }
            Divider()
            HStack {
                SettingsTabLink(tab: .shortcuts, title: "更改快捷键…")
                Spacer()
                Button("完成") { dismiss() }.keyboardShortcut(.cancelAction)
            }
            .padding(12)
        }
        .frame(width: 560, height: 620)
    }

    private func section(_ title: String, _ rows: [(String, String)]) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title).font(.headline)
            Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 4) {
                ForEach(Array(rows.enumerated()), id: \.offset) { _, row in
                    GridRow {
                        Text(row.0).monospacedDigit().foregroundStyle(.secondary).gridColumnAlignment(.trailing).frame(minWidth: 120, alignment: .trailing)
                        Text(row.1)
                    }
                }
            }
        }
    }
}
