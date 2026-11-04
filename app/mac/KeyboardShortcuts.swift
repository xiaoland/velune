import AppKit
import SwiftUI

/// A key combination as the shortcut settings keep it.
struct Shortcut: Codable, Hashable {
    struct Modifiers: OptionSet, Codable, Hashable {
        let rawValue: Int
        static let command = Modifiers(rawValue: 1)
        static let shift = Modifiers(rawValue: 2)
        static let option = Modifiers(rawValue: 4)
        static let control = Modifiers(rawValue: 8)
    }
    /// The keys without a character of their own, by the names kept for them.
    private static let named: [String: (key: KeyEquivalent, glyph: String, code: UInt16)] = [
        "return": (.return, "↩", 36), "escape": (.escape, "⎋", 53), "delete": (.delete, "⌫", 51), "tab": (.tab, "⇥", 48),
        "space": (.space, "Space", 49), "leftArrow": (.leftArrow, "←", 123), "rightArrow": (.rightArrow, "→", 124),
        "downArrow": (.downArrow, "↓", 125), "upArrow": (.upArrow, "↑", 126), "home": (.home, "↖", 115),
        "end": (.end, "↘", 119), "pageUp": (.pageUp, "⇞", 116), "pageDown": (.pageDown, "⇟", 121),
    ]

    /// A character as the key types it without modifiers, or a name of `named`.
    var key: String
    var modifiers: Modifiers

    init(_ key: String, _ modifiers: Modifiers = .command) {
        self.key = key
        self.modifiers = modifiers
    }

    static func code(named key: String) -> UInt16? { named[key]?.code }

    /// The combination a key press makes; nil for a lone modifier or a key
    /// with neither a character nor a name here.
    init?(event: NSEvent) {
        let flags = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        var modifiers: Modifiers = []
        if flags.contains(.command) { modifiers.insert(.command) }
        if flags.contains(.shift) { modifiers.insert(.shift) }
        if flags.contains(.option) { modifiers.insert(.option) }
        if flags.contains(.control) { modifiers.insert(.control) }
        let code = event.keyCode == 76 ? 36 : event.keyCode
        if let name = Self.named.first(where: { $0.value.code == code })?.key {
            self.init(name, modifiers)
        } else if let character = event.characters(byApplyingModifiers: [])?.lowercased(), character.count == 1,
                  let scalar = character.unicodeScalars.first, !CharacterSet.controlCharacters.contains(scalar),
                  !(0xF700...0xF8FF).contains(scalar.value) {
            self.init(character, modifiers)
        } else {
            return nil
        }
    }

    /// Typing must still reach text: anything but Escape needs Command,
    /// Control or Option.
    var isUsable: Bool { key == "escape" || !modifiers.isDisjoint(with: [.command, .control, .option]) }

    var keyboardShortcut: KeyboardShortcut {
        var events: EventModifiers = []
        if modifiers.contains(.command) { events.insert(.command) }
        if modifiers.contains(.shift) { events.insert(.shift) }
        if modifiers.contains(.option) { events.insert(.option) }
        if modifiers.contains(.control) { events.insert(.control) }
        return KeyboardShortcut(Self.named[key]?.key ?? KeyEquivalent(Character(key)), modifiers: events)
    }

    /// As menus show it: ⌃⌥⇧⌘ then the key.
    var title: String {
        var text = ""
        if modifiers.contains(.control) { text += "⌃" }
        if modifiers.contains(.option) { text += "⌥" }
        if modifiers.contains(.shift) { text += "⇧" }
        if modifiers.contains(.command) { text += "⌘" }
        return text + (Self.named[key]?.glyph ?? key.uppercased())
    }
}

/// How the composer sends: Return, with Shift-Return for a new line, or
/// Command-Return, with Return for a new line. While a turn runs the
/// sending key queues and Command (Option-Command) with Return steers.
enum SendMode: String, Codable, CaseIterable, Identifiable {
    case returnSends, commandReturnSends
    var id: Self { self }
    var title: String {
        switch self {
        case .returnSends: "Return 发送，⇧Return 换行"
        case .commandReturnSends: "⌘Return 发送，Return 换行"
        }
    }
    /// The key that sends or queues, as the composer's hints name it.
    var sendKey: String { self == .returnSends ? "Return" : "⌘Return" }
    /// The key that steers into the running turn.
    var steerKey: String { self == .returnSends ? "⌘Return" : "⌥⌘Return" }
}

extension View {
    /// The send mode's key on a sheet's primary button, so a message edited
    /// there is sent as one typed in the composer; the other key is a new line.
    func sendKeyboardShortcut() -> some View {
        keyboardShortcut(KeyboardShortcutSettings.shared.sendMode == .returnSends ? .defaultAction : KeyboardShortcut(.return, modifiers: .command))
    }
}

/// Velune's commands the user may give other shortcuts, with the ones they
/// come with. Sending and steering stay with the composer's own keys (see
/// `SendMode`); the global ones work while another app is active.
enum ShortcutCommand: String, CaseIterable, Identifiable {
    case newConversation, openConversation, newTab, newWindow, reopenClosedTab, closeTab, closeWindow
    case find
    case stop, stopSubAgents, continueTurn, compactContext, sideChat, attach, chooseRuntime, chooseModel, chooseEffort, choosePermission, chooseWorkingDirectory, nextAttention, reload
    case splitRight, splitDown, closeSplit, back, forward, nextPane, previousPane, nextTab, previousTab, moveTabToNewWindow, keepInFront
    case commandPalette, focusTranscript, showList, showOutline, focusComposer, showAgents, rightArea, bottomArea, editorOnly, problems
    case shortcutHelp
    case globalShow, globalNewConversation

    var id: Self { self }

    /// The menu it is in, which the settings group it by.
    enum Menu: String, CaseIterable, Identifiable {
        case file = "文件", edit = "编辑", conversation = "会话", window = "窗口", view = "显示", help = "帮助", global = "全局"
        var id: Self { self }
        var sectionTitle: String { self == .global ? "全局（在其他应用中也有效）" : "\(rawValue)菜单" }
    }

    var menu: Menu {
        switch self {
        case .newConversation, .openConversation, .newTab, .newWindow, .reopenClosedTab, .closeTab, .closeWindow: .file
        case .find: .edit
        case .stop, .stopSubAgents, .continueTurn, .compactContext, .sideChat, .attach, .chooseRuntime, .chooseModel, .chooseEffort, .choosePermission, .chooseWorkingDirectory, .nextAttention, .reload: .conversation
        case .splitRight, .splitDown, .closeSplit, .back, .forward, .nextPane, .previousPane, .nextTab, .previousTab, .moveTabToNewWindow, .keepInFront: .window
        case .commandPalette, .focusTranscript, .showList, .showOutline, .focusComposer, .showAgents, .rightArea, .bottomArea, .editorOnly, .problems: .view
        case .shortcutHelp: .help
        case .globalShow, .globalNewConversation: .global
        }
    }

    var isGlobal: Bool { menu == .global }

    var title: String {
        switch self {
        case .newConversation: "新建会话"
        case .openConversation: "打开会话…"
        case .newTab: "新建标签页"
        case .newWindow: "新建窗口"
        case .reopenClosedTab: "重新打开关闭的标签页"
        case .closeTab: "关闭标签页"
        case .closeWindow: "关闭窗口"
        case .find: "查找…"
        case .stop: "停止生成"
        case .stopSubAgents: "停止子 Agent"
        case .continueTurn: "继续"
        case .compactContext: "压缩上下文"
        case .sideChat: "侧聊"
        case .attach: "添加附件…"
        case .chooseRuntime: "选择运行时…"
        case .chooseModel: "选择模型…"
        case .chooseEffort: "选择推理强度…"
        case .choosePermission: "选择权限模式…"
        case .chooseWorkingDirectory: "选择工作目录…"
        case .nextAttention: "转到待处理的会话"
        case .reload: "重新读取"
        case .splitRight: "向右拆分"
        case .splitDown: "向下拆分"
        case .closeSplit: "关闭分屏"
        case .back: "后退"
        case .forward: "前进"
        case .nextPane: "下一个分屏"
        case .previousPane: "上一个分屏"
        case .nextTab: "下一个标签页"
        case .previousTab: "上一个标签页"
        case .moveTabToNewWindow: "将标签页移到新窗口"
        case .keepInFront: "保持在最前面"
        case .commandPalette: "命令…"
        case .focusTranscript: "转到消息"
        case .showList: "显示／隐藏会话列表"
        case .showOutline: "显示／隐藏会话大纲"
        case .focusComposer: "转到输入区"
        case .showAgents: "显示／隐藏关联 Agent"
        case .rightArea: "显示／隐藏右侧区域"
        case .bottomArea: "显示／隐藏底部区域"
        case .editorOnly: "只显示编辑区"
        case .problems: "问题…"
        case .shortcutHelp: "快捷键速查"
        case .globalShow: "显示 Velune"
        case .globalNewConversation: "新建会话（全局）"
        }
    }

    /// The English name the palette and the `/` list also find the command
    /// by; never shown (DESIGN.md, 检索别名). Every command has one.
    var englishName: String {
        switch self {
        case .newConversation: "New Conversation"
        case .openConversation: "Open Conversation"
        case .newTab: "New Tab"
        case .newWindow: "New Window"
        case .reopenClosedTab: "Reopen Closed Tab"
        case .closeTab: "Close Tab"
        case .closeWindow: "Close Window"
        case .find: "Find"
        case .stop: "Stop"
        case .stopSubAgents: "Stop Sub-agents"
        case .continueTurn: "Continue"
        case .compactContext: "Compact context"
        case .sideChat: "Side Chat"
        case .attach: "Attach Files"
        case .chooseRuntime: "Choose Runtime"
        case .chooseModel: "Choose Model"
        case .chooseEffort: "Choose Reasoning Effort"
        case .choosePermission: "Choose Permission Mode"
        case .chooseWorkingDirectory: "Choose Working Directory"
        case .nextAttention: "Go to Next Conversation Needing Attention"
        case .reload: "Reload"
        case .splitRight: "Split Right"
        case .splitDown: "Split Down"
        case .closeSplit: "Close Split"
        case .back: "Back"
        case .forward: "Forward"
        case .nextPane: "Next Pane"
        case .previousPane: "Previous Pane"
        case .nextTab: "Next Tab"
        case .previousTab: "Previous Tab"
        case .moveTabToNewWindow: "Move Tab to New Window"
        case .keepInFront: "Keep in Front"
        case .commandPalette: "Command Palette"
        case .focusTranscript: "Go to Messages"
        case .showList: "Toggle Conversation List"
        case .showOutline: "Toggle Outline"
        case .focusComposer: "Go to Composer"
        case .showAgents: "Toggle Linked Agents"
        case .rightArea: "Toggle Right Area"
        case .bottomArea: "Toggle Bottom Area"
        case .editorOnly: "Editor Only"
        case .problems: "Problems"
        case .shortcutHelp: "Keyboard Shortcuts"
        case .globalShow: "Show Velune"
        case .globalNewConversation: "New Conversation (Global)"
        }
    }

    /// Other words people look for the command by, in either language.
    var keywords: [String] {
        switch self {
        case .newConversation: ["chat", "session"]
        case .openConversation: ["quick open", "switch conversation", "切换会话"]
        case .find: ["search", "搜索"]
        case .stop: ["cancel", "interrupt", "abort", "中断", "取消"]
        case .stopSubAgents: ["subagent", "cancel"]
        case .continueTurn: ["resume", "retry", "重试"]
        case .compactContext: ["compact", "压缩"]
        case .sideChat: ["btw", "side question", "aside", "旁问", "顺便问"]
        case .attach: ["attachment", "upload", "file", "image", "文件", "图片"]
        case .chooseRuntime: ["switch runtime", "agent", "harness", "切换运行时"]
        case .chooseModel: ["switch model", "llm", "切换模型"]
        case .chooseEffort: ["reasoning", "thinking", "切换推理强度", "思考"]
        case .choosePermission: ["approval", "sandbox", "切换权限", "审批"]
        case .chooseWorkingDirectory: ["cwd", "directory", "folder", "path", "change directory", "切换目录", "路径", "文件夹"]
        case .nextAttention: ["pending", "waiting", "approval", "unread", "待处理", "未读"]
        case .reload: ["refresh", "刷新"]
        case .splitRight, .splitDown, .closeSplit, .nextPane, .previousPane: ["pane", "分屏"]
        case .keepInFront: ["always on top", "pin", "置顶"]
        case .commandPalette: ["commands"]
        case .focusTranscript: ["transcript", "消息列表"]
        case .showList: ["sidebar", "侧栏"]
        case .focusComposer: ["input", "prompt", "输入框"]
        case .showAgents: ["subagent", "子 Agent"]
        case .editorOnly: ["zen", "maximize", "专注"]
        case .problems: ["errors", "diagnostics", "issues", "错误"]
        case .shortcutHelp: ["keys", "keybindings", "cheat sheet", "help", "快捷键"]
        default: []
        }
    }

    /// Everything the command is found by besides its title: its English
    /// name, its keywords and its `/` name.
    var searchTerms: [String] {
        [englishName] + keywords + ComposerSlashCommand.all.filter { $0.command == self }.map(\.name)
    }

    var standard: Shortcut? {
        switch self {
        case .newConversation: Shortcut("n")
        // As VS Code's Quick Open.
        case .openConversation: Shortcut("p")
        case .newTab: Shortcut("t")
        case .newWindow: Shortcut("n", [.command, .option])
        // As in browsers and VS Code.
        case .reopenClosedTab: Shortcut("t", [.command, .shift])
        case .closeTab: Shortcut("w")
        case .closeWindow: Shortcut("w", [.command, .shift])
        case .find: Shortcut("f")
        case .stop: Shortcut("escape", [])
        case .reload: Shortcut("r")
        case .attach: Shortcut("o")
        // Control-Command and a letter: the next turn's choices.
        case .chooseRuntime: Shortcut("r", [.command, .control])
        case .chooseModel: Shortcut("m", [.command, .control])
        case .chooseEffort: Shortcut("e", [.command, .control])
        case .choosePermission: Shortcut("p", [.command, .control])
        case .nextAttention: Shortcut("a", [.command, .shift])
        case .commandPalette: Shortcut("k")
        case .focusTranscript: Shortcut("0")
        case .shortcutHelp: Shortcut("/")
        case .splitRight: Shortcut("d")
        case .splitDown: Shortcut("d", [.command, .shift])
        // Control-brackets, not Option-arrows: those move by word in the composer.
        case .back: Shortcut("[", .control)
        case .forward: Shortcut("]", .control)
        case .nextPane: Shortcut("]")
        case .previousPane: Shortcut("[")
        // As in Chrome; Control-Command-arrows often switch desktops.
        case .nextTab: Shortcut("rightArrow", [.command, .option])
        case .previousTab: Shortcut("leftArrow", [.command, .option])
        case .showList: Shortcut("1")
        case .showOutline: Shortcut("2")
        case .focusComposer: Shortcut("3")
        case .showAgents: Shortcut("4")
        // As in VS Code; the left area keeps the system's Show Sidebar.
        case .rightArea: Shortcut("b", [.command, .option])
        case .bottomArea: Shortcut("j")
        case .editorOnly: Shortcut("return", [.command, .shift])
        case .problems: Shortcut("m", [.command, .shift])
        // Global ones are the user's to choose: any default would take a key from other apps.
        case .stopSubAgents, .continueTurn, .compactContext, .sideChat, .chooseWorkingDirectory, .closeSplit, .moveTabToNewWindow, .keepInFront, .globalShow, .globalNewConversation: nil
        }
    }

    /// The command showing or hiding a tool window from the View menu.
    static func tool(_ kind: ToolWindowKind) -> ShortcutCommand {
        switch kind {
        case .list: .showList
        case .outline: .showOutline
        case .composer: .focusComposer
        case .agents: .showAgents
        }
    }
}

/// The shortcuts of Velune's commands, those the user changed kept in
/// `VELUNE_HOME/mac-shortcuts.json`. Any other schema or an unreadable file
/// starts from the standard ones and is replaced on the next change.
@MainActor final class KeyboardShortcutSettings: ObservableObject {
    static let shared = KeyboardShortcutSettings()

    private struct File: Codable {
        static let schema = 1
        var schema = File.schema
        /// By command; null where the user removed the shortcut.
        var shortcuts: [String: Shortcut?]
        /// Absent in files from before the choice: Return sends.
        var sendMode: SendMode?
    }

    /// The menu items of the system and of AppKit a shortcut must leave alone.
    static let reserved: [Shortcut: String] = [
        Shortcut("q"): "退出 Velune", Shortcut("h"): "隐藏 Velune", Shortcut("h", [.command, .option]): "隐藏其他",
        Shortcut(","): "设置", Shortcut("m"): "最小化", Shortcut("z"): "撤销", Shortcut("z", [.command, .shift]): "重做",
        Shortcut("x"): "剪切", Shortcut("c"): "拷贝", Shortcut("v"): "粘贴", Shortcut("a"): "全选",
        Shortcut("s", [.command, .control]): "显示边栏", Shortcut("f", [.command, .control]): "进入全屏幕",
        Shortcut("return"): "发送消息与插入本轮", Shortcut("return", [.command, .option]): "插入本轮",
        Shortcut("upArrow", .option): "编辑最后一条排队消息",
    ]

    /// The user's changes; a command not here has its standard shortcut.
    @Published private(set) var changed: [ShortcutCommand: Shortcut?] = [:]
    @Published private(set) var sendMode = SendMode.returnSends
    private var url: URL?
    private var failed: ((Error) -> Void)?

    /// Reads the user's changes from `home`; without one they stay for this run.
    func load(home: URL?, failed: @escaping (Error) -> Void) {
        url = home?.appendingPathComponent("mac-shortcuts.json")
        self.failed = failed
        let file = url.flatMap { try? Data(contentsOf: $0) }.flatMap { try? JSONDecoder().decode(File.self, from: $0) }
        guard let file, file.schema == File.schema else { return }
        sendMode = file.sendMode ?? .returnSends
        changed = Dictionary(uniqueKeysWithValues: file.shortcuts.compactMap { name, shortcut in
            ShortcutCommand(rawValue: name).map { ($0, shortcut) }
        })
    }

    func shortcut(_ command: ShortcutCommand) -> Shortcut? {
        if let shortcut = changed[command] { shortcut } else { command.standard }
    }

    subscript(command: ShortcutCommand) -> KeyboardShortcut? { shortcut(command)?.keyboardShortcut }

    /// `text` with the command's shortcut after it, for a button's help.
    func help(_ text: String, _ command: ShortcutCommand) -> String {
        shortcut(command).map { "\(text)（\($0.title)）" } ?? text
    }

    /// Who already has `shortcut`: another command, or the system.
    func holder(of shortcut: Shortcut, except command: ShortcutCommand) -> String? {
        if let name = Self.reserved[shortcut] { return name }
        return ShortcutCommand.allCases.first { $0 != command && self.shortcut($0) == shortcut }.map(\.title)
    }

    func set(_ shortcut: Shortcut?, for command: ShortcutCommand) {
        if shortcut == command.standard { changed[command] = nil } else { changed[command] = .some(shortcut) }
        write()
    }

    func reset(_ command: ShortcutCommand) {
        changed[command] = nil
        write()
    }

    func resetAll() {
        changed = [:]
        write()
    }

    func setSendMode(_ mode: SendMode) {
        guard mode != sendMode else { return }
        sendMode = mode
        write()
    }

    private func write() {
        guard let url else { return }
        do {
            try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            let file = File(shortcuts: Dictionary(uniqueKeysWithValues: changed.map { ($0.key.rawValue, $0.value) }), sendMode: sendMode)
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
            try encoder.encode(file).write(to: url, options: .atomic)
        } catch { failed?(error) }
    }
}

/// The general settings' sections listing the commands with their shortcuts.
struct ShortcutSettingsSections: View {
    @ObservedObject private var shortcuts = KeyboardShortcutSettings.shared
    @ObservedObject private var hotKeys = GlobalHotKeys.shared
    @State private var recording: ShortcutCommand?
    /// Why the last press did not record, shown under the row recording it.
    @State private var refusal: Refusal?

    var body: some View {
        Group {
            Section("输入区") {
                Picker("发送方式", selection: Binding(get: { shortcuts.sendMode }, set: { shortcuts.setSendMode($0) })) {
                    ForEach(SendMode.allCases) { Text($0.title).tag($0) }
                }
                Text("运行中按 \(shortcuts.sendMode.sendKey) 排队，按 \(shortcuts.sendMode.steerKey) 插入本轮；运行时不支持插入时改为排队。输入框为空时 ↑↓ 调出本会话之前的消息，⌥↑ 取回最后一条排队消息。")
                    .foregroundStyle(.secondary).uiFont(.callout)
            }
            Section("快捷键") {
                HStack {
                    Text("点按快捷键后按下新的组合，按 ⌫ 移除，再点按一次取消。")
                        .foregroundStyle(.secondary).uiFont(.callout)
                    Spacer()
                    Button("全部恢复默认") { shortcuts.resetAll(); refusal = nil }.disabled(shortcuts.changed.isEmpty)
                }
            }
            ForEach(ShortcutCommand.Menu.allCases) { menu in
                Section(menu.sectionTitle) {
                    ForEach(ShortcutCommand.allCases.filter { $0.menu == menu }) { command in
                        LabeledContent {
                            HStack(spacing: 6) {
                                if shortcuts.changed[command] != nil {
                                    Button { shortcuts.reset(command); refusal = nil } label: { Image(systemName: "arrow.uturn.backward") }
                                        .buttonStyle(.borderless).help(command.standard.map { "恢复为 \($0.title)" } ?? "恢复为无快捷键")
                                }
                                ShortcutRecorder(command: command, recording: $recording, refusal: $refusal)
                            }
                        } label: {
                            Text(command.title)
                            if let refusal, refusal.command == command { Text(refusal.reason).foregroundStyle(.red) }
                            else if hotKeys.failed.contains(command) { Text("系统或其他应用已占用这个组合，未生效。").foregroundStyle(.red) }
                        }
                    }
                }
            }
        }
        .onDisappear { recording = nil }
    }
}

/// A key press a command could not take, and why.
private struct Refusal: Equatable {
    let command: ShortcutCommand
    let reason: String
}

/// A command's shortcut; clicked, it takes the next key press as the new one.
private struct ShortcutRecorder: View {
    let command: ShortcutCommand
    @Binding var recording: ShortcutCommand?
    @Binding var refusal: Refusal?
    @ObservedObject private var shortcuts = KeyboardShortcutSettings.shared
    @State private var monitor: Any?

    private var isRecording: Bool { recording == command }

    var body: some View {
        Button { recording = isRecording ? nil : command } label: {
            Text(isRecording ? "按下快捷键…" : shortcuts.shortcut(command)?.title ?? "无")
                .monospacedDigit().frame(minWidth: 96)
                .foregroundStyle(isRecording ? Color.accentColor : shortcuts.shortcut(command) == nil ? .secondary : .primary)
        }
        .onChange(of: isRecording) { _, now in now ? start() : stop() }
        .onDisappear(perform: stop)
    }

    private func start() {
        refusal = nil
        // Ahead of the menus, so the press records instead of running a command.
        monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
            take(event)
            return nil
        }
    }

    private func stop() {
        if let monitor { NSEvent.removeMonitor(monitor) }
        monitor = nil
    }

    private func take(_ event: NSEvent) {
        guard let shortcut = Shortcut(event: event) else { NSSound.beep(); return }
        if shortcut == Shortcut("delete", []) {
            shortcuts.set(nil, for: command)
        } else if !shortcut.isUsable {
            refusal = Refusal(command: command, reason: "\(shortcut.title) 会挡住输入：请加上 ⌘、⌃ 或 ⌥。")
            NSSound.beep()
            return
        } else if let holder = shortcuts.holder(of: shortcut, except: command) {
            refusal = Refusal(command: command, reason: "\(shortcut.title) 已用于“\(holder)”。")
            NSSound.beep()
            return
        } else {
            shortcuts.set(shortcut, for: command)
        }
        recording = nil
    }
}
