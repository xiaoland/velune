import AppKit
import SwiftUI

/// The list over the composer's text while a `/` command or an `@` path is
/// typed: ↑↓ choose, Return or Tab takes the choice, Escape closes the list.
struct ComposerCompletion: Equatable {
    enum Kind: Equatable { case command, file }
    struct Item: Equatable, Identifiable {
        let id: String
        let title: String
        var detail: String? = nil
        var shortcut: String? = nil
        var enabled = true
    }
    let kind: Kind
    /// The typed `/name` or `@path` a chosen item replaces.
    let range: NSRange
    var items: [Item]
    var selected = 0
}

/// Velune's commands as `/` names them; any other `/…` goes to the runtime as typed.
enum ComposerSlashCommand {
    static let all: [(name: String, command: ShortcutCommand)] = [
        ("model", .chooseModel), ("effort", .chooseEffort), ("permission", .choosePermission), ("runtime", .chooseRuntime),
        ("new", .newConversation), ("continue", .continueTurn), ("compact", .compactContext), ("stop", .stop), ("attach", .attach),
        ("open", .openConversation), ("find", .find), ("commands", .commandPalette), ("keys", .shortcutHelp),
    ]
}

/// The files of a working directory relative to it, read in the background
/// and kept for a minute: Git's tracked and unignored files in a repository,
/// else a bounded walk that skips hidden and build folders.
@MainActor final class WorkingDirectoryFiles {
    static let shared = WorkingDirectoryFiles()
    private var cache: [String: (files: [String], read: Date)] = [:]
    private var reading: Set<String> = []
    nonisolated private static let limit = 50_000

    /// The files as last read, nil before the first read ends; `ready` runs when a read ends.
    func files(in directory: String, ready: @escaping () -> Void) -> [String]? {
        let cached = cache[directory]
        if cached.map({ Date().timeIntervalSince($0.read) > 60 }) ?? true, !reading.contains(directory) {
            reading.insert(directory)
            DispatchQueue.global(qos: .userInitiated).async {
                let files = Self.read(directory)
                DispatchQueue.main.async {
                    self.reading.remove(directory)
                    self.cache[directory] = (files, Date())
                    ready()
                }
            }
        }
        return cached?.files
    }

    nonisolated private static func read(_ directory: String) -> [String] {
        var url = URL(fileURLWithPath: directory)
        var inRepository = false
        while url.path != "/" {
            if FileManager.default.fileExists(atPath: url.appendingPathComponent(".git").path) { inRepository = true; break }
            url.deleteLastPathComponent()
        }
        if inRepository, let files = gitFiles(directory) { return files }
        return walk(directory)
    }

    nonisolated private static func gitFiles(_ directory: String) -> [String]? {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/git")
        process.arguments = ["-C", directory, "ls-files", "-co", "--exclude-standard", "-z"]
        let output = Pipe()
        process.standardOutput = output
        process.standardError = FileHandle.nullDevice
        do { try process.run() } catch { return nil }
        let data = output.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        guard process.terminationStatus == 0 else { return nil }
        return String(decoding: data, as: UTF8.self).split(separator: "\0").prefix(limit).map(String.init)
    }

    nonisolated private static func walk(_ directory: String) -> [String] {
        let root = URL(fileURLWithPath: directory, isDirectory: true)
        let skipped: Set<String> = ["node_modules", "target", "build", "dist", "DerivedData", "Pods", "vendor"]
        guard let enumerator = FileManager.default.enumerator(at: root, includingPropertiesForKeys: [.isDirectoryKey], options: [.skipsHiddenFiles, .skipsPackageDescendants]) else { return [] }
        var files: [String] = []
        let prefix = root.standardizedFileURL.path.count + 1
        for case let url as URL in enumerator {
            if (try? url.resourceValues(forKeys: [.isDirectoryKey]))?.isDirectory == true {
                if skipped.contains(url.lastPathComponent) { enumerator.skipDescendants() }
                continue
            }
            files.append(String(url.standardizedFileURL.path.dropFirst(prefix)))
            if files.count >= limit { break }
        }
        return files
    }
}

extension ComposerModel {
    /// The `/command` the box holds alone, or the `@path` the caret is in,
    /// with what matches it; none while composing or after Escape closed it.
    func updateCompletion() {
        // The view's own text: its selection moves before the edit reaches `text`.
        guard let view = textView, !view.hasMarkedText(), view.selectedRange().length == 0, view.string != dismissedCompletionText else { return setCompletion(nil) }
        let text = view.string
        let string = text as NSString
        let caret = min(view.selectedRange().location, string.length)
        if text.hasPrefix("/"), text.rangeOfCharacter(from: .whitespacesAndNewlines) == nil {
            let query = String(text.dropFirst()).lowercased()
            let shortcuts = KeyboardShortcutSettings.shared
            // The name first, then the title and the command's other names, so `/模型` finds `/model` too.
            let matched = query.isEmpty ? ComposerSlashCommand.all : ComposerSlashCommand.all.enumerated()
                .compactMap { index, entry in FuzzyMatch.score(query, [entry.name, entry.command.title] + entry.command.searchTerms).map { (entry, $0, index) } }
                .sorted { $0.1 != $1.1 ? $0.1 > $1.1 : $0.2 < $1.2 }.map(\.0)
            let items = matched.map { entry in
                ComposerCompletion.Item(id: entry.name, title: "/" + entry.name, detail: entry.command.title,
                                        shortcut: shortcuts.shortcut(entry.command)?.title, enabled: canPerform?(entry.command) ?? false)
            }
            return setCompletion(items.isEmpty ? nil : ComposerCompletion(kind: .command, range: NSRange(location: 0, length: string.length), items: items))
        }
        // The word before the caret, when it starts with `@` after a space or the start.
        var start = caret
        while start > 0, let scalar = UnicodeScalar(string.character(at: start - 1)), !CharacterSet.whitespacesAndNewlines.contains(scalar) { start -= 1 }
        guard start < caret, string.character(at: start) == 64, let directory = fileDirectory else { return setCompletion(nil) }
        let query = string.substring(with: NSRange(location: start + 1, length: caret - start - 1))
        guard let files = WorkingDirectoryFiles.shared.files(in: directory, ready: { [weak self] in self?.updateCompletion() }) else {
            return setCompletion(ComposerCompletion(kind: .file, range: NSRange(location: start, length: caret - start), items: [.init(id: "", title: "正在读取工作目录…", enabled: false)]))
        }
        let items = Self.rank(files, query).map { path in
            ComposerCompletion.Item(id: path, title: (path as NSString).lastPathComponent, detail: (path as NSString).deletingLastPathComponent)
        }
        setCompletion(items.isEmpty ? nil : ComposerCompletion(kind: .file, range: NSRange(location: start, length: caret - start), items: items))
    }

    /// The best 50 paths for `query`, the file's name weighing more than its folders; shorter first among equals.
    private static func rank(_ files: [String], _ query: String) -> [String] {
        guard !query.isEmpty else { return Array(files.prefix(50)) }
        var scored: [(path: String, score: Int)] = []
        for path in files {
            if let score = FuzzyMatch.score(query, [(path as NSString).lastPathComponent, path]) { scored.append((path, score)) }
        }
        scored.sort { $0.score != $1.score ? $0.score > $1.score : $0.path.count < $1.path.count }
        return scored.prefix(50).map(\.path)
    }

    /// Where `@` finds files: the conversation's working directory on this Mac.
    private var fileDirectory: String? {
        guard let store, store.loadedConversationLocation.host == nil else { return nil }
        return store.loadedConversationWorkingDirectory
    }

    /// The composer's keys while the list shows; returns whether one was taken.
    func completionKey(_ event: NSEvent) -> Bool {
        guard var value = completion else { return false }
        let modifiers = event.modifierFlags.intersection([.command, .shift, .option, .control])
        guard modifiers.isEmpty else { return false }
        switch event.keyCode {
        case 126, 125:
            value.selected = (value.selected + (event.keyCode == 126 ? -1 : 1) + value.items.count) % value.items.count
            completion = value
        case 36, 76, 48: acceptCompletion(at: value.selected)
        default: return false
        }
        return true
    }

    func acceptCompletion(at index: Int) {
        guard let value = completion, value.items.indices.contains(index), let view = textView else { return }
        let item = value.items[index]
        guard item.enabled else { NSSound.beep(); return }
        switch value.kind {
        case .command:
            guard let command = ComposerSlashCommand.all.first(where: { $0.name == item.id })?.command else { return }
            view.insertText("", replacementRange: value.range)
            setCompletion(nil)
            perform?(command)
        case .file:
            // The path as typed text; the agent reads the file itself.
            view.insertText(item.id + " ", replacementRange: value.range)
        }
    }

    func setCompletion(_ value: ComposerCompletion?) {
        if var value, let current = completion, current.kind == value.kind, current.range.location == value.range.location {
            // Typing on keeps the chosen row while it still matches.
            value.selected = value.items.firstIndex { $0.id == current.items[safe: current.selected]?.id } ?? 0
            if value != completion { completion = value }
        } else if value != completion {
            completion = value
        }
        if completion != nil, escapeToken == nil {
            escapeToken = KeyRouter.addEscapeHandler { [weak self] in
                guard let self, completion != nil, let view = textView, view.window?.firstResponder === view else { return false }
                dismissedCompletionText = view.string
                setCompletion(nil)
                return true
            }
        } else if completion == nil, let token = escapeToken {
            KeyRouter.removeEscapeHandler(token)
            escapeToken = nil
        }
    }
}

extension Array {
    subscript(safe index: Int) -> Element? { indices.contains(index) ? self[index] : nil }
}

/// The rows of a completion, above the composer's text.
struct ComposerCompletionList: View {
    let completion: ComposerCompletion
    let choose: (Int) -> Void
    private static let rowHeight: CGFloat = 24
    /// Up to six rows, then it scrolls.
    static func height(_ completion: ComposerCompletion) -> CGFloat { min(CGFloat(completion.items.count), 6) * rowHeight + 8 }

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(spacing: 0) {
                    ForEach(Array(completion.items.enumerated()), id: \.element.id) { index, item in
                        let selected = index == completion.selected
                        HStack(spacing: 8) {
                            Text(item.title).lineLimit(1)
                            if let detail = item.detail, !detail.isEmpty {
                                Text(detail).lineLimit(1).truncationMode(.head)
                                    .foregroundStyle(selected ? AnyShapeStyle(.white.opacity(0.8)) : AnyShapeStyle(.secondary))
                            }
                            Spacer(minLength: 8)
                            if let shortcut = item.shortcut {
                                Text(shortcut).foregroundStyle(selected ? AnyShapeStyle(.white.opacity(0.8)) : AnyShapeStyle(.secondary))
                            }
                        }
                        .foregroundStyle(selected ? AnyShapeStyle(.white) : item.enabled ? AnyShapeStyle(.primary) : AnyShapeStyle(.tertiary))
                        .padding(.horizontal, 8).frame(height: Self.rowHeight)
                        .background(selected ? Color.accentColor : .clear, in: RoundedRectangle(cornerRadius: 5))
                        .contentShape(Rectangle())
                        .onTapGesture { choose(index) }
                    }
                }.padding(4)
            }
            .frame(height: Self.height(completion))
            .onChange(of: completion.selected) { _, value in if let item = completion.items[safe: value] { proxy.scrollTo(item.id) } }
        }
        .background(Color(nsColor: .textBackgroundColor), in: RoundedRectangle(cornerRadius: ComposerEditor.cornerRadius))
        .overlay(RoundedRectangle(cornerRadius: ComposerEditor.cornerRadius).strokeBorder(Color(nsColor: .separatorColor)))
        .accessibilityLabel(completion.kind == .command ? "命令" : "文件")
    }
}

/// The `/` or `@` list in a borderless window of its own just above the
/// composer's text: it covers what is above rather than making the
/// composer's area taller, and never takes the keyboard from the text.
@MainActor final class ComposerCompletionPanel {
    private final class Panel: NSPanel {
        override var canBecomeKey: Bool { false }
        override var canBecomeMain: Bool { false }
    }
    /// A click on a row chooses it at once; the window is never key.
    private final class Hosting: NSHostingView<ComposerCompletionList> {
        override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
    }
    private var panel: Panel?
    private var hosting: Hosting?
    private weak var anchor: NSView?
    private var resizing: NSObjectProtocol?

    func show(_ completion: ComposerCompletion?, anchor: NSView?, choose: @escaping (Int) -> Void) {
        guard let completion, let anchor, let window = anchor.window, !anchor.isHiddenOrHasHiddenAncestor else { return hide() }
        let list = ComposerCompletionList(completion: completion, choose: choose)
        if let hosting {
            hosting.rootView = list
        } else {
            let hosting = Hosting(rootView: list)
            let panel = Panel(contentRect: .zero, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: true)
            panel.isOpaque = false
            panel.backgroundColor = .clear
            panel.hasShadow = true
            panel.contentView = hosting
            self.hosting = hosting
            self.panel = panel
        }
        guard let panel else { return }
        self.anchor = anchor
        if panel.parent !== window {
            panel.parent?.removeChildWindow(panel)
            window.addChildWindow(panel, ordered: .above)
            if let resizing { NotificationCenter.default.removeObserver(resizing) }
            resizing = NotificationCenter.default.addObserver(forName: NSWindow.didResizeNotification, object: window, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.place() }
            }
        }
        place()
        // Again once the composer's layout settles: a taller text moves its top.
        DispatchQueue.main.async { [weak self] in self?.place() }
    }

    func hide() {
        guard let panel else { return }
        panel.parent?.removeChildWindow(panel)
        panel.orderOut(nil)
        if let resizing { NotificationCenter.default.removeObserver(resizing) }
        resizing = nil
    }

    private func place() {
        guard let panel, panel.parent != nil, let hosting, let anchor, let window = anchor.window else { return }
        let field = window.convertToScreen(anchor.convert(anchor.bounds, to: nil))
        panel.setFrame(NSRect(x: field.minX, y: field.maxY + 4, width: field.width, height: ComposerCompletionList.height(hosting.rootView.completion)), display: true)
    }
}
