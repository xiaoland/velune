import AppKit
import SwiftUI

/// What the palette over a window lists: Velune's commands, the
/// conversations to open, or one of the next turn's choices.
enum PaletteMode: Hashable {
    case commands, conversations, runtime, model, effort, permission

    var prompt: String {
        switch self {
        case .commands: "输入命令"
        case .conversations: "按标题、项目或工作目录查找会话"
        case .runtime: "下一轮的运行时"
        case .model: "下一轮的模型"
        case .effort: "下一轮的推理强度"
        case .permission: "下一轮的权限模式"
        }
    }
}

/// One row of the palette. `run` receives whether ⌘Return chose it, which
/// opens a conversation in a new tab. `aliases` are what it is also found
/// by, never shown (DESIGN.md, 检索别名).
struct PaletteItem: Identifiable {
    let id: String
    let title: String
    var aliases: [String] = []
    var detail: String? = nil
    var shortcut: String? = nil
    var checked = false
    var enabled = true
    let run: (_ newTab: Bool) -> Void
}

/// Matching typed text against titles, in order but not necessarily
/// together: each word must match, and earlier, tighter matches rank higher.
/// Fields come title first, then aliases, then detail, so a title match
/// outranks the same match in an alias.
enum FuzzyMatch {
    static func score(_ query: String, _ fields: [String]) -> Int? {
        let words = query.lowercased().split(whereSeparator: \.isWhitespace).map(String.init)
        guard !words.isEmpty else { return 0 }
        let texts = fields.map { $0.lowercased() }
        var total = 0
        for word in words {
            guard let best = texts.enumerated().compactMap({ index, text in score(word, text).map { $0 - index * 50 } }).max() else { return nil }
            total += best
        }
        return total
    }

    private static func score(_ word: String, _ text: String) -> Int? {
        if text.hasPrefix(word) { return 3000 - text.count }
        if let range = text.range(of: word) { return 2000 - text.distance(from: text.startIndex, to: range.lowerBound) }
        var gaps = 0
        var index = text.startIndex
        for character in word {
            guard let found = text[index...].firstIndex(of: character) else { return nil }
            gaps += text.distance(from: index, to: found)
            index = text.index(after: found)
        }
        return 1000 - gaps
    }
}

extension ConversationWindowModel {
    func showPalette(_ mode: PaletteMode) {
        if palette == mode { return closePalette() }
        if palette == nil {
            // A field's editor stands in for the field, which is what takes the keyboard back.
            let responder = (NSApp.keyWindow ?? window)?.firstResponder
            paletteReturnResponder = (responder as? NSTextView).flatMap { $0.isFieldEditor ? $0.delegate as? NSResponder : nil } ?? responder
        }
        palette = mode
    }

    /// Typing goes back where it was, or to the composer after a choice for the next turn.
    func closePalette(focusComposer: Bool = false) {
        guard palette != nil else { return }
        palette = nil
        let responder = paletteReturnResponder
        paletteReturnResponder = nil
        DispatchQueue.main.async { [weak self] in
            if !focusComposer, let view = responder as? NSView, let window = view.window, window.makeFirstResponder(view) { return }
            self?.focused.composer.focusWhenShown()
        }
    }

    private var store: AppStore { focused.store }

    /// Whether `command` does something now; the menus and the palette both ask.
    func canPerform(_ command: ShortcutCommand, composerCommand: Bool = false) -> Bool {
        switch command {
        case .newConversation: !store.isLoading
        case .newWindow: openWindow != nil
        case .openConversation, .newTab, .closeTab, .closeWindow, .find, .splitRight, .splitDown, .commandPalette, .focusTranscript, .rightArea, .bottomArea, .editorOnly, .problems, .shortcutHelp, .nextAttention: true
        case .stop: store.canCancel && !store.isLoading
        case .stopSubAgents: store.followUp.canStopSubAgents
        case .continueTurn: composerCommand ? focused.canResumeAfterCommand : focused.submission.canResume
        case .attach: !store.isShuttingDown
        case .chooseRuntime: !store.enabledRuntimeInstances.isEmpty && !store.isShuttingDown
        case .chooseModel: store.canSwitchModel
        case .chooseEffort: !store.reasoningEffortOptions.isEmpty
        case .choosePermission: !store.permissionModeOptions.isEmpty
        case .reload: store.canReloadFromRuntimes
        case .closeSplit, .nextPane, .previousPane: isSplit
        case .back: !store.backHistory.isEmpty
        case .forward: !store.forwardHistory.isEmpty
        case .nextTab, .previousTab: focusedPane.tabs.count > 1
        case .moveTabToNewWindow: canMoveTabToNewWindow
        case .keepInFront: WindowPinning.shared.canPin
        case .showList: canToggle(.list)
        case .showOutline: canToggle(.outline)
        case .focusComposer: canToggle(.composer)
        case .showAgents: canToggle(.agents)
        case .globalShow, .globalNewConversation: false
        }
    }

    func perform(_ command: ShortcutCommand) {
        guard canPerform(command) else { return }
        switch command {
        case .newConversation: store.createConversation()
        case .openConversation: showPalette(.conversations)
        case .newTab: openInNewTab()
        case .newWindow: openWindow?(.workspace, WindowSeed())
        case .closeTab: closeTab(focused.id)
        case .closeWindow: NSApp.keyWindow?.performClose(nil)
        case .find: find()
        case .stop: store.cancel()
        case .stopSubAgents: store.stopSubAgents()
        case .continueTurn: focused.resume()
        case .attach: focused.composer.chooseAttachments()
        case .chooseRuntime: showPalette(.runtime)
        case .chooseModel: showPalette(.model)
        case .chooseEffort: showPalette(.effort)
        case .choosePermission: showPalette(.permission)
        case .nextAttention: goToNextAttention()
        case .reload: store.reloadFromRuntimes()
        case .splitRight: split(.horizontal)
        case .splitDown: split(.vertical)
        case .closeSplit: closePane(focusedPaneID)
        case .back: focused.goBack()
        case .forward: focused.goForward()
        case .nextPane: focusNeighbour(forward: true)
        case .previousPane: focusNeighbour(forward: false)
        case .nextTab: selectNeighbourTab(forward: true)
        case .previousTab: selectNeighbourTab(forward: false)
        case .moveTabToNewWindow: moveToNewWindow(focused.id)
        case .keepInFront: WindowPinning.shared.toggle()
        case .commandPalette: showPalette(.commands)
        case .focusTranscript: focused.focusTranscript()
        case .showList: toggle(.list)
        case .showOutline: toggle(.outline)
        case .focusComposer: focusComposer()
        case .showAgents: toggle(.agents)
        case .rightArea: tools.toggleArea(.right)
        case .bottomArea: tools.toggleArea(.bottom)
        case .editorOnly: tools.toggleEditorOnly()
        case .problems: openScene?("problems")
        case .shortcutHelp: showsShortcutHelp = true
        case .globalShow, .globalNewConversation: break
        }
    }

    /// The next conversation that needs the user: a request waiting for an
    /// answer, then a turn that ended unseen, then one that failed unseen;
    /// the latest first in each. A request in the shown conversation takes
    /// the keyboard first.
    func goToNextAttention() {
        let current = store.loadedConversationID
        let waiting = application.waitingConversationIDs
        if !store.pendingInteractions.isEmpty { focused.focusInteraction(); return }
        let listed = list.conversations.sorted { ($0.updatedAtUnixMs ?? 0) > ($1.updatedAtUnixMs ?? 0) }
        let ordered = listed.filter { waiting.contains($0.id) } + listed.filter { $0.unread && !$0.failed && !waiting.contains($0.id) }
            + listed.filter { $0.unread && $0.failed && !waiting.contains($0.id) }
        guard let next = ordered.first(where: { $0.id != current }) else { NSSound.beep(); return }
        open(next.id)
        if waiting.contains(next.id) { focused.focusInteraction() } else { focused.composer.focusWhenShown() }
    }

    /// What “沿用当前” and “运行时默认” are also found by.
    private static let unchosenAliases = ["current", "default", "keep", "none"]

    /// The rows for `mode`, in the order shown before typing narrows them.
    func paletteItems(_ mode: PaletteMode) -> [PaletteItem] {
        let shortcuts = KeyboardShortcutSettings.shared
        switch mode {
        case .commands:
            let commands = ShortcutCommand.allCases.filter { !$0.isGlobal && $0 != .commandPalette }.map { command in
                PaletteItem(id: command.rawValue, title: command.title.replacingOccurrences(of: "…", with: ""), aliases: command.searchTerms, detail: command.menu.rawValue,
                            shortcut: shortcuts.shortcut(command)?.title, enabled: canPerform(command)) { [weak self] _ in
                    guard let self else { return }
                    // A command that opens another list keeps the palette for it.
                    let opensList: Set<ShortcutCommand> = [.openConversation, .chooseRuntime, .chooseModel, .chooseEffort, .choosePermission]
                    if opensList.contains(command) { perform(command) } else { closePalette(); perform(command) }
                }
            }
            let settings: [(SettingsTab, String, String)] = [(.general, "通用", "General"), (.conversation, "会话", "Conversations"), (.shortcuts, "快捷键", "Keyboard Shortcuts"),
                                                              (.providers, "AI 提供商", "AI Providers"), (.routes, "模型路由", "Model Routes"), (.runtimes, "Agent 运行时", "Agent Runtimes")]
                + (DeviceHub.shared == nil ? [] : [(.devices, "设备", "Devices")])
            let pages = settings.map { tab, name, english in
                PaletteItem(id: "settings:\(name)", title: "设置：\(name)", aliases: ["Settings: " + english, "preferences"], detail: "设置") { [weak self] _ in
                    self?.closePalette()
                    SettingsNavigator.shared.tab = tab
                    self?.openSettings?()
                }
            }
            let windows = [("analytics", "分析", ["Analytics", "usage", "tokens", "用量"]), ("resource-library", "资源库", ["Resource Library", "skills", "mcp", "instructions", "技能"])].map { id, name, aliases in
                PaletteItem(id: "window:\(id)", title: name, aliases: aliases, detail: "窗口") { [weak self] _ in self?.closePalette(); self?.openScene?(id) }
            }
            let queued = store.followUp.queue.last.map { _ in
                [PaletteItem(id: "queue:last", title: "编辑最后一条排队消息", aliases: ["Edit Last Queued Message", "queue"], detail: "输入区", shortcut: "⌥↑") { [weak self] _ in
                    self?.closePalette(focusComposer: true)
                    DispatchQueue.main.async { _ = self?.focused.composer.takeLastQueued() }
                }]
            } ?? []
            return commands + queued + pages + windows
        case .conversations:
            let runtimes = Dictionary(list.runtimeInstances.map { ($0.id, $0.name) }, uniquingKeysWith: { first, _ in first })
            let waiting = application.waitingConversationIDs
            return list.conversations.filter { $0.parentConversationID == nil }.map { conversation in
                let place = conversation.project ?? conversation.cwd
                let state = waiting.contains(conversation.id) ? "等待回应" : list.isRunning(conversation) ? "运行中" : conversation.failed ? "失败" : conversation.unread ? "未读" : nil
                let detail = [place.map { URL(fileURLWithPath: $0).lastPathComponent }, runtimes[conversation.runtimeID], state].compactMap { $0 }.joined(separator: " · ")
                return PaletteItem(id: conversation.id, title: conversation.title.isEmpty ? "未命名会话" : conversation.title, detail: detail,
                                   checked: conversation.id == store.loadedConversationID) { [weak self] newTab in
                    guard let self else { return }
                    closePalette(focusComposer: true)
                    if newTab { openInNewTab(conversation.id) } else { open(conversation.id) }
                }
            }
        case .runtime:
            return store.enabledRuntimeInstances.map { runtime in
                PaletteItem(id: runtime.id, title: runtime.name, aliases: [runtime.typeID], checked: runtime.id == store.nextTurnRuntimeID) { [weak self] _ in
                    self?.closePalette(focusComposer: true)
                    self?.store.selectNextTurnRuntime(runtime.id)
                }
            }
        case .model:
            let none = store.nextTurnHasCurrent && store.nextTurnModel != nil ? "沿用当前（\(store.selectedModelName ?? "")）" : "运行时默认"
            let choices: [(String, String?, ModelSelection?)] = [(none, nil, nil)] + store.modelChoices.map { choice in
                if case .managed = choice.selection { (choice.displayName, "Velune 提供商", choice.selection) } else { (choice.displayName, nil, choice.selection) }
            }
            return choices.enumerated().map { index, choice in
                let aliases: [String] = switch choice.2 {
                case .native(_, let modelID)?: [modelID]
                case .managed?: []
                case nil: Self.unchosenAliases
                }
                return PaletteItem(id: "\(index):\(choice.0)", title: choice.0, aliases: aliases, detail: choice.1, checked: store.chosenModel == choice.2) { [weak self] _ in
                    self?.closePalette(focusComposer: true)
                    self?.store.selectModel(choice.2)
                }
            }
        case .effort:
            let none = store.currentEffort.map { "沿用当前（\($0)）" } ?? (store.nextTurnHasCurrent ? "沿用当前" : "运行时默认")
            return ([nil] + store.reasoningEffortOptions.map(Optional.some)).map { level in
                PaletteItem(id: level ?? "", title: level ?? none, aliases: level == nil ? Self.unchosenAliases : [], checked: store.nextTurnEffort == level) { [weak self] _ in
                    self?.closePalette(focusComposer: true)
                    self?.store.selectEffort(level)
                }
            }
        case .permission:
            let current = store.currentPermissionMode
            let none = current.map { "沿用当前（\($0.label)）" } ?? (store.nextTurnHasCurrent ? "沿用当前" : "运行时默认")
            let chosen = store.effectivePermissionMode?.id
            return [PaletteItem(id: "", title: none, aliases: Self.unchosenAliases, checked: chosen == nil) { [weak self] _ in self?.closePalette(focusComposer: true); self?.store.selectPermissionMode(nil) }]
                + store.permissionModeOptions.map { option in
                    PaletteItem(id: option.id, title: option.label, aliases: [option.id], detail: option.detail, checked: chosen == option.id) { [weak self] _ in
                        self?.closePalette(focusComposer: true)
                        self?.store.selectPermissionMode(option.id)
                    }
                }
        }
    }
}

/// The palette over the top of a window: a field and the rows it narrows.
/// ↑↓ choose, Return runs (⌘Return opens a conversation in a new tab),
/// Escape or a click outside closes it.
struct CommandPaletteView: View {
    @ObservedObject var window: ConversationWindowModel
    let mode: PaletteMode
    @ObservedObject private var store: AppStore
    @ObservedObject private var composer: ComposerModel

    init(window: ConversationWindowModel, mode: PaletteMode) {
        self.window = window; self.mode = mode
        store = window.focused.store; composer = window.focused.composer
    }

    @State private var query = ""
    @State private var selection = 0
    private static let rowHeight: CGFloat = 28

    private var items: [PaletteItem] {
        let all = window.paletteItems(mode)
        guard !query.isEmpty else { return all }
        return all.enumerated().compactMap { index, item in FuzzyMatch.score(query, [item.title] + item.aliases + (item.detail.map { [$0] } ?? [])).map { (item, $0, index) } }
            .sorted { $0.1 != $1.1 ? $0.1 > $1.1 : $0.2 < $1.2 }.map(\.0)
    }

    var body: some View {
        let items = items
        VStack(spacing: 0) {
            PaletteField(text: $query, prompt: mode.prompt,
                         move: { step in if !items.isEmpty { selection = (selection + step + items.count) % items.count } },
                         run: { newTab in run(items, newTab: newTab) },
                         cancel: { window.closePalette() })
                .frame(height: 28).padding(.horizontal, 12).padding(.vertical, 8)
            Divider()
            if items.isEmpty {
                Text(mode == .commands ? "没有匹配的命令" : mode == .conversations ? "没有匹配的会话" : "没有可选的项")
                    .foregroundStyle(.secondary).frame(maxWidth: .infinity).padding(14)
            } else {
                ScrollViewReader { proxy in
                    ScrollView {
                        LazyVStack(spacing: 0) {
                            ForEach(Array(items.enumerated()), id: \.element.id) { index, item in
                                row(item, selected: index == selection)
                                    .contentShape(Rectangle())
                                    .onTapGesture { selection = index; run(items, newTab: NSEvent.modifierFlags.contains(.command)) }
                            }
                        }.padding(6)
                    }
                    .frame(height: min(CGFloat(items.count), 12) * Self.rowHeight + 12)
                    .onChange(of: selection) { _, value in if items.indices.contains(value) { proxy.scrollTo(items[value].id) } }
                }
            }
        }
        .frame(width: 580)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).strokeBorder(Color(nsColor: .separatorColor)))
        .shadow(color: .black.opacity(0.25), radius: 24, y: 10)
        .onChange(of: query) { _, _ in selection = 0 }
        .onAppear { selection = items.firstIndex { $0.checked } ?? 0 }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(mode.prompt)
    }

    private func row(_ item: PaletteItem, selected: Bool) -> some View {
        HStack(spacing: 8) {
            if mode != .commands {
                Image(systemName: "checkmark").opacity(item.checked ? 1 : 0).frame(width: 14)
            }
            Text(item.title).lineLimit(1).truncationMode(.middle)
            if let detail = item.detail {
                Text(detail).lineLimit(1).truncationMode(.middle)
                    .foregroundStyle(selected ? AnyShapeStyle(.white.opacity(0.8)) : AnyShapeStyle(.secondary))
            }
            Spacer(minLength: 8)
            if let shortcut = item.shortcut {
                Text(shortcut).monospacedDigit().foregroundStyle(selected ? AnyShapeStyle(.white.opacity(0.8)) : AnyShapeStyle(.secondary))
            }
        }
        .foregroundStyle(selected ? AnyShapeStyle(.white) : item.enabled ? AnyShapeStyle(.primary) : AnyShapeStyle(.tertiary))
        .padding(.horizontal, 8).frame(height: Self.rowHeight)
        .background(selected ? Color.accentColor : .clear, in: RoundedRectangle(cornerRadius: 6, style: .continuous))
        .accessibilityAddTraits(selected ? .isSelected : [])
    }

    private func run(_ items: [PaletteItem], newTab: Bool) {
        guard items.indices.contains(selection) else { return }
        let item = items[selection]
        guard item.enabled else { NSSound.beep(); return }
        item.run(newTab)
        query = ""
    }
}

/// The palette's text field: it keeps the keyboard and hands ↑↓, Return,
/// ⌘Return and Escape to the palette instead of moving its caret.
private struct PaletteField: NSViewRepresentable {
    @Binding var text: String
    let prompt: String
    let move: (Int) -> Void
    let run: (_ newTab: Bool) -> Void
    let cancel: () -> Void

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    func makeNSView(context: Context) -> PaletteTextField {
        let field = PaletteTextField()
        field.isBordered = false
        field.drawsBackground = false
        field.focusRingType = .none
        field.font = .systemFont(ofSize: 18)
        field.placeholderString = prompt
        field.delegate = context.coordinator
        field.coordinator = context.coordinator
        field.setAccessibilityLabel(prompt)
        return field
    }

    func updateNSView(_ field: PaletteTextField, context: Context) {
        context.coordinator.parent = self
        if field.stringValue != text { field.stringValue = text }
        if field.placeholderString != prompt { field.placeholderString = prompt }
    }

    @MainActor final class Coordinator: NSObject, NSTextFieldDelegate {
        var parent: PaletteField
        init(_ parent: PaletteField) { self.parent = parent }
        func controlTextDidChange(_ notification: Notification) {
            parent.text = (notification.object as? NSTextField)?.stringValue ?? ""
        }
        func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
            switch selector {
            case #selector(NSResponder.moveUp(_:)): parent.move(-1)
            case #selector(NSResponder.moveDown(_:)): parent.move(1)
            case #selector(NSResponder.insertNewline(_:)): parent.run(false)
            case #selector(NSResponder.cancelOperation(_:)): parent.cancel()
            default: return false
            }
            return true
        }
    }
}

private final class PaletteTextField: NSTextField {
    weak var coordinator: PaletteField.Coordinator?

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        DispatchQueue.main.async { [weak self] in
            guard let self, let window else { return }
            window.makeFirstResponder(self)
        }
    }

    /// ⌘Return would reach the menus' Send first.
    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        if event.keyCode == 36 || event.keyCode == 76, event.modifierFlags.intersection([.command, .shift, .option, .control]) == .command,
           let editor = currentEditor(), window?.firstResponder === editor {
            coordinator?.parent.run(true)
            return true
        }
        return super.performKeyEquivalent(with: event)
    }
}
