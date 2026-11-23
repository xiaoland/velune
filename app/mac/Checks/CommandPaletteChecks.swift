#if VELUNE_DEV_CHECKS
import AppKit
import SwiftUI

/// Production text field in an unshown host. This exercises AppKit marked text
/// and SwiftUI updates; real input-method candidate windows need a visible check.
@MainActor func commandPaletteChecks() -> Never {
    NSApplication.shared.setActivationPolicy(.prohibited)
    func settle() { RunLoop.main.run(until: Date().addingTimeInterval(0.1)) }
    func field(in view: NSView) -> PaletteTextField? {
        if let field = view as? PaletteTextField { return field }
        return view.subviews.lazy.compactMap { field(in: $0) }.first
    }
    let state = PaletteCheckState()
    let host = NSHostingView(rootView: PaletteCheckHost(state: state))
    let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 580, height: 100),
                          styleMask: .borderless, backing: .buffered, defer: false)
    window.contentView = host
    host.layoutSubtreeIfNeeded()
    settle()
    let input = field(in: host)!
    precondition(window.makeFirstResponder(input))
    let editor = input.currentEditor() as! NSTextView
    let coordinator = input.coordinator!
    let commands = [#selector(NSResponder.moveUp(_:)), #selector(NSResponder.moveDown(_:)),
                    #selector(NSResponder.insertNewline(_:)), #selector(NSResponder.cancelOperation(_:))]
    for marked in ["n", "ni", "ni'hao"] {
        editor.setMarkedText(marked, selectedRange: NSRange(location: marked.utf16.count, length: 0),
                             replacementRange: NSRange(location: NSNotFound, length: 0))
        let range = editor.markedRange()
        // Runtime/list updates can redraw the palette before a text-change
        // notification. This used to clear marked text on the first keystroke.
        state.revision += 1
        settle()
        precondition(editor.hasMarkedText() && editor.markedRange() == range && editor.string == marked,
                     "无关视图更新不得取消输入法组字")
        precondition(window.firstResponder === editor && field(in: host) === input)
        editor.didChangeText()
        settle()
        precondition(state.query.isEmpty && editor.hasMarkedText(), "未提交的拼音不得触发筛选")
        for command in commands {
            precondition(!coordinator.control(input, textView: editor, doCommandBy: command),
                         "组字中的方向键、回车和Escape交还输入法")
        }
        precondition(state.actions.isEmpty)
    }
    editor.insertText("你好", replacementRange: editor.markedRange())
    editor.didChangeText()
    settle()
    precondition(!editor.hasMarkedText() && state.query == "你好", "上屏后同步中文筛选词")
    precondition(FuzzyMatch.score(state.query, ["你好，世界"]) != nil)
    for command in commands { precondition(coordinator.control(input, textView: editor, doCommandBy: command)) }
    precondition(state.actions == ["up", "down", "run", "cancel"])

    state.query = ""
    settle()
    precondition(input.stringValue.isEmpty && editor.string.isEmpty, "提交后允许外部清空搜索词")
    editor.insertText("English", replacementRange: NSRange(location: 0, length: 0))
    editor.didChangeText()
    settle()
    precondition(state.query == "English")
    state.actions.removeAll()
    let commandReturn = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: .command,
        timestamp: 0, windowNumber: window.windowNumber, context: nil,
        characters: "\r", charactersIgnoringModifiers: "\r", isARepeat: false, keyCode: 36)!
    precondition(input.performKeyEquivalent(with: commandReturn) && state.actions == ["run-new-tab"])

    // A second composition starts after committed Latin text, with an existing
    // query. Cmd-Return must not escape into the palette or the Send menu.
    state.actions.removeAll()
    editor.setSelectedRange(NSRange(location: editor.string.utf16.count, length: 0))
    editor.setMarkedText("zhong", selectedRange: NSRange(location: 5, length: 0),
                         replacementRange: NSRange(location: NSNotFound, length: 0))
    state.revision += 1
    settle()
    precondition(editor.hasMarkedText() && state.query == "English")
    precondition(input.performKeyEquivalent(with: commandReturn) && state.actions.isEmpty)

    FileHandle.standardOutput.write(Data("Palette AppKit marked-text checks passed; no OS IME keyboard input was posted.\n".utf8))
    exit(0)
}

@MainActor private final class PaletteCheckState: ObservableObject {
    @Published var query = ""
    @Published var revision = 0
    var actions: [String] = []
}

private struct PaletteCheckHost: View {
    @ObservedObject var state: PaletteCheckState
    var body: some View {
        VStack {
            PaletteField(text: $state.query, prompt: "查找会话",
                move: { state.actions.append($0 < 0 ? "up" : "down") },
                run: { state.actions.append($0 ? "run-new-tab" : "run") },
                cancel: { state.actions.append("cancel") })
            Text("状态更新：\(state.revision)")
        }.frame(width: 580, height: 100)
    }
}
#endif
