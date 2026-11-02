import AppKit
import VeluneBindings

/// The menu and commands present the same native panels, even while the
/// composer is hidden. Confirmation keeps the conversation and runtime it opened for.
extension ConversationWindowModel {
    func canChooseLocalWorkingDirectory(_ input: ComposerContext) -> Bool {
        guard device == nil, let runtime = input.nextTurnRuntimeID else { return false }
        return input.store.device.runtimeLocations[runtime] == .local
    }

    func chooseWorkingDirectory(_ input: ComposerContext) {
        guard input.canChangeWorkingDirectory, canChooseLocalWorkingDirectory(input) else { return }
        let conversation = input.store.loadedConversationID
        let panel = NSOpenPanel()
        panel.title = "选择新会话的工作目录"
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        if let path = input.store.loadedConversationWorkingDirectory { panel.directoryURL = URL(fileURLWithPath: path, isDirectory: true) }
        if panel.runModal() == .OK, let url = panel.url {
            applyWorkingDirectory(url.standardizedFileURL.path, to: input, conversation: conversation)
        }
    }

    func enterWorkingDirectory(_ input: ComposerContext) {
        guard input.canChangeWorkingDirectory, let owner = workingDirectoryWindow(input) else { return }
        let conversation = input.store.loadedConversationID
        let local = canChooseLocalWorkingDirectory(input)
        let alert = NSAlert()
        alert.messageText = "输入工作目录"
        if let host = input.store.device.location(of: input.nextTurnRuntimeID).host {
            alert.informativeText = "请输入 \(host) 上的已有绝对目录；发送时检查。"
        } else if device != nil {
            alert.informativeText = "请输入目标设备上的已有绝对目录；发送时检查。"
        } else {
            alert.informativeText = "请输入已有目录的绝对路径，也可使用 ~/ 开头的路径；发送时检查。"
        }
        let use = alert.addButton(withTitle: "使用")
        alert.addButton(withTitle: "取消").keyEquivalent = "\u{1b}"
        let field = NSTextField(frame: NSRect(x: 0, y: 0, width: 460, height: 24))
        field.placeholderString = "绝对路径"
        field.setAccessibilityLabel("绝对路径")
        field.stringValue = input.store.loadedConversationWorkingDirectory ?? ""
        let validation = DirectoryPathValidation(field: field, button: use)
        field.delegate = validation
        use.isEnabled = !validation.path.isEmpty
        alert.accessoryView = field
        alert.window.initialFirstResponder = field
        alert.beginSheetModal(for: owner) { response in
            guard response == .alertFirstButtonReturn, !validation.path.isEmpty else { return }
            let path = local ? (validation.path as NSString).expandingTildeInPath : validation.path
            self.applyWorkingDirectory(path, to: input, conversation: conversation)
        }
    }

    private func workingDirectoryWindow(_ input: ComposerContext) -> NSWindow? {
        input.model.textView?.window ?? window?.attachedSheet ?? window
    }

    private func applyWorkingDirectory(_ path: String, to input: ComposerContext, conversation: String?) {
        let current = composerContext(for: input.tab)
        guard current.store.loadedConversationID == conversation,
              current.nextTurnRuntimeID == input.nextTurnRuntimeID else { return }
        current.selectWorkingDirectory(path)
    }
}

/// NSAlert retains the field; the completion retains its validation delegate
/// until dismissal, so an empty path disables the default action while editing.
@MainActor private final class DirectoryPathValidation: NSObject, NSTextFieldDelegate {
    let field: NSTextField
    let button: NSButton
    var path: String { field.stringValue.trimmingCharacters(in: .whitespacesAndNewlines) }
    init(field: NSTextField, button: NSButton) { self.field = field; self.button = button }
    func controlTextDidChange(_ notification: Notification) { button.isEnabled = !path.isEmpty }
}
