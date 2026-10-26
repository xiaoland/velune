import SwiftUI

struct ConversationKeyboardActions {
    var send: (() -> Void)?
    var stop: (() -> Void)?
    var focus: () -> Void
    var latest: () -> Void
    var outline: () -> Void
    var agents: () -> Void
}
struct WorkspaceKeyboardActions {
    var create: (() -> Void)?
    var search: () -> Void
    var back: (() -> Void)?
}
extension FocusedValues {
    @Entry var conversationActions: ConversationKeyboardActions?
    @Entry var workspaceActions: WorkspaceKeyboardActions?
    @Entry var newWorkspaceWindow: (() -> Void)?
    @Entry var showProblems: (() -> Void)?
}

struct WorkspaceCommands: Commands {
    @FocusedValue(\.conversationActions) private var conversation
    @FocusedValue(\.workspaceActions) private var workspace
    @FocusedValue(\.newWorkspaceWindow) private var newWindow
    @FocusedValue(\.showProblems) private var problems
    var body: some Commands {
        CommandGroup(replacing: .newItem) {
            Button("新会话") { workspace?.create?() }.keyboardShortcut("n").disabled(workspace?.create == nil)
            Button("新窗口") { newWindow?() }.keyboardShortcut("n", modifiers: [.command, .shift]).disabled(newWindow == nil)
        }
        CommandMenu("会话") {
            Button("搜索会话") { workspace?.search() }.keyboardShortcut("f").disabled(workspace == nil)
            Button("聚焦输入") { conversation?.focus() }.keyboardShortcut("l").disabled(conversation == nil)
            Button("发送消息") { conversation?.send?() }.keyboardShortcut(.return, modifiers: .command).disabled(conversation?.send == nil)
            Button("停止生成") { conversation?.stop?() }.keyboardShortcut(".").disabled(conversation?.stop == nil)
            Divider()
            Button("大纲") { conversation?.outline() }.keyboardShortcut("o", modifiers: [.command, .shift]).disabled(conversation == nil)
            Button("关联 Agent") { conversation?.agents() }.keyboardShortcut("a", modifiers: [.command, .shift]).disabled(conversation == nil)
            Button("问题") { problems?() }.keyboardShortcut("m", modifiers: [.command, .shift]).disabled(problems == nil)
            Divider()
            Button("返回上个会话") { workspace?.back?() }.keyboardShortcut("[").disabled(workspace?.back == nil)
            Button("回到最新") { conversation?.latest() }.keyboardShortcut(.downArrow, modifiers: [.command, .shift]).disabled(conversation == nil)
        }
    }
}
