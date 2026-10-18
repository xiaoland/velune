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

/// Velune's commands the user may give other shortcuts, with the ones they
/// come with. Sending keeps Command-Return: the composer takes it itself.
enum ShortcutCommand: String, CaseIterable, Identifiable {
    case newConversation, newTab, newWindow, closeTab, closeWindow
    case find
    case stop, stopSubAgents, reload
    case splitRight, splitDown, closeSplit, back, forward, nextPane, previousPane, nextTab, previousTab, moveTabToNewWindow, keepInFront
    case showList, showOutline, focusComposer, showAgents, rightArea, bottomArea, editorOnly, problems

    var id: Self { self }

    /// The menu it is in, which the settings group it by.
    enum Menu: String, CaseIterable, Identifiable {
        case file = "文件", edit = "编辑", conversation = "会话", window = "窗口", view = "显示"
        var id: Self { self }
    }

    var menu: Menu {
        switch self {
        case .newConversation, .newTab, .newWindow, .closeTab, .closeWindow: .file
        case .find: .edit
        case .stop, .stopSubAgents, .reload: .conversation
        case .splitRight, .splitDown, .closeSplit, .back, .forward, .nextPane, .previousPane, .nextTab, .previousTab, .moveTabToNewWindow, .keepInFront: .window
        case .showList, .showOutline, .focusComposer, .showAgents, .rightArea, .bottomArea, .editorOnly, .problems: .view
        }
    }

    var title: String {
        switch self {
        case .newConversation: "新建会话"
        case .newTab: "新建标签页"
        case .newWindow: "新建窗口"
        case .closeTab: "关闭标签页"
        case .closeWindow: "关闭窗口"
        case .find: "查找…"
        case .stop: "停止生成"
        case .stopSubAgents: "停止子 Agent"
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
        case .showList: "显示／隐藏会话列表"
        case .showOutline: "显示／隐藏会话大纲"
        case .focusComposer: "转到输入区"
        case .showAgents: "显示／隐藏关联 Agent"
        case .rightArea: "显示／隐藏右侧区域"
        case .bottomArea: "显示／隐藏底部区域"
        case .editorOnly: "只显示编辑区"
        case .problems: "问题…"
        }
    }

    var standard: Shortcut? {
        switch self {
        case .newConversation: Shortcut("n")
        case .newTab: Shortcut("t")
        case .newWindow: Shortcut("n", [.command, .option])
        case .closeTab: Shortcut("w")
        case .closeWindow: Shortcut("w", [.command, .shift])
        case .find: Shortcut("f")
        case .stop: Shortcut("escape", [])
        case .reload: Shortcut("r")
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
        case .stopSubAgents, .closeSplit, .moveTabToNewWindow, .keepInFront: nil
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
    }

    /// The menu items of the system and of AppKit a shortcut must leave alone.
    static let reserved: [Shortcut: String] = [
        Shortcut("q"): "退出 Velune", Shortcut("h"): "隐藏 Velune", Shortcut("h", [.command, .option]): "隐藏其他",
        Shortcut(","): "设置", Shortcut("m"): "最小化", Shortcut("z"): "撤销", Shortcut("z", [.command, .shift]): "重做",
        Shortcut("x"): "剪切", Shortcut("c"): "拷贝", Shortcut("v"): "粘贴", Shortcut("a"): "全选",
        Shortcut("s", [.command, .control]): "显示边栏", Shortcut("f", [.command, .control]): "进入全屏幕",
        Shortcut("return"): "发送消息", Shortcut("return", [.command, .option]): "插话",
    ]

    /// The user's changes; a command not here has its standard shortcut.
    @Published private(set) var changed: [ShortcutCommand: Shortcut?] = [:]
    private var url: URL?
    private var failed: ((Error) -> Void)?

    /// Reads the user's changes from `home`; without one they stay for this run.
    func load(home: URL?, failed: @escaping (Error) -> Void) {
        url = home?.appendingPathComponent("mac-shortcuts.json")
        self.failed = failed
        let file = url.flatMap { try? Data(contentsOf: $0) }.flatMap { try? JSONDecoder().decode(File.self, from: $0) }
        guard let file, file.schema == File.schema else { return }
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

    private func write() {
        guard let url else { return }
        do {
            try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            let file = File(shortcuts: Dictionary(uniqueKeysWithValues: changed.map { ($0.key.rawValue, $0.value) }))
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
            try encoder.encode(file).write(to: url, options: .atomic)
        } catch { failed?(error) }
    }
}

/// The general settings' sections listing the commands with their shortcuts.
struct ShortcutSettingsSections: View {
    @ObservedObject private var shortcuts = KeyboardShortcutSettings.shared
    @State private var recording: ShortcutCommand?
    /// Why the last press did not record, shown under the row recording it.
    @State private var refusal: Refusal?

    var body: some View {
        Group {
            Section("快捷键") {
                HStack {
                    Text("点按快捷键后按下新的组合，按 ⌫ 移除，再点按一次取消。发送消息固定为 ⌘Return。")
                        .foregroundStyle(.secondary).font(.callout)
                    Spacer()
                    Button("全部恢复默认") { shortcuts.resetAll(); refusal = nil }.disabled(shortcuts.changed.isEmpty)
                }
            }
            ForEach(ShortcutCommand.Menu.allCases) { menu in
                Section("\(menu.rawValue)菜单") {
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
