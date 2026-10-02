import AppKit

// Thin local client. Rust owns SQLite, routing, and simulation execution.
final class AppDelegate: NSObject, NSApplicationDelegate {
    var window: NSWindow!
    let text = NSTextView()
    let status = NSTextField(labelWithString: "Host 未连接 · 仅模拟，无真实模型")
    var timer: Timer?
    var busy = false
    var stopped = false
    var lastReport: [String: Any] = [:]
    let worker = DispatchQueue(label: "velune.local.ipc")
    var binary: URL { Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers/velune-core") }
    var stateDir: URL { FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0].appendingPathComponent("VelunePrototype") }

    func applicationDidFinishLaunching(_ notification: Notification) {
        let menu = NSMenu()
        let appItem = NSMenuItem(); menu.addItem(appItem)
        let appMenu = NSMenu(); appItem.submenu = appMenu
        appMenu.addItem(withTitle: "退出界面（Host 继续运行）", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        NSApplication.shared.mainMenu = menu
        window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 960, height: 700), styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
        window.title = "Velune · 本地模拟原型 0.1.0"
        let stack = NSStackView(); stack.orientation = .vertical; stack.spacing = 12; stack.edgeInsets = NSEdgeInsets(top: 18, left: 18, bottom: 18, right: 18)
        let title = NSTextField(labelWithString: "Mac 本地任务与跨 Harness 协作")
        title.font = .boldSystemFont(ofSize: 22)
        stack.addArrangedSubview(title)
        stack.addArrangedSubview(NSTextField(labelWithString: "模拟模式：Codex → Claude Code / Pi → Codex · 不读取账号，不调用模型"))
        stack.addArrangedSubview(status)
        let buttons = NSStackView(); buttons.orientation = .horizontal; buttons.spacing = 8
        for (label, action) in [("连接 / 启动 Host", #selector(start)), ("运行样例", #selector(run)), ("取消待执行", #selector(cancel)), ("安全停止 Host", #selector(stop)), ("导出反馈", #selector(exportFeedback))] {
            let button = NSButton(title: label, target: self, action: action); buttons.addArrangedSubview(button)
        }
        stack.addArrangedSubview(buttons)
        text.isEditable = false; text.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
        text.isVerticallyResizable = true; text.autoresizingMask = [.width]
        let scroll = NSScrollView(); scroll.hasVerticalScroller = true; scroll.documentView = text
        stack.addArrangedSubview(scroll)
        scroll.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -36).isActive = true
        scroll.heightAnchor.constraint(greaterThanOrEqualToConstant: 400).isActive = true
        stack.addArrangedSubview(NSTextField(labelWithString: "关闭界面不会停止 Host。取消只影响未投递消息；未知状态不会自动重试。"))
        window.contentView = stack; window.center(); window.makeKeyAndOrderFront(nil)
        NSApplication.shared.activate(ignoringOtherApps: true)
        start()
        timer = Timer.scheduledTimer(withTimeInterval: 0.6, repeats: true) { [weak self] _ in
            guard let self = self, !self.stopped else { return }; self.send("status")
        }
    }
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }
    // Process remains owned by the OS after UI termination. No launch agent / autostart install.
    func rpc(_ command: String) throws -> [String: Any] {
        let process = Process(); process.executableURL = binary
        process.arguments = ["rpc", stateDir.path, command]
        let output = Pipe(); let errors = Pipe(); process.standardOutput = output; process.standardError = errors
        try process.run()
        let data = output.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        guard process.terminationStatus == 0 else { throw NSError(domain: "Velune", code: Int(process.terminationStatus), userInfo: [NSLocalizedDescriptionKey: "Host 未连接或 IPC 请求失败；可点击连接 / 启动 Host。"] ) }
        guard let value = try JSONSerialization.jsonObject(with: data) as? [String: Any] else { throw NSError(domain: "Velune", code: 1) }
        return value
    }
    @objc func start() {
        guard !busy else { return }; busy = true; stopped = false
        worker.async {
            do {
                if (try? self.rpc("status")) == nil {
                    try FileManager.default.createDirectory(at: self.stateDir, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
                    let host = Process(); host.executableURL = self.binary; host.arguments = ["host", self.stateDir.path]
                    host.standardInput = FileHandle.nullDevice; host.standardOutput = FileHandle.nullDevice; host.standardError = FileHandle.nullDevice
                    try host.run()
                    // Bounded startup retries; UI remains responsive.
                    for _ in 0..<20 { if (try? self.rpc("status")) != nil { break }; Thread.sleep(forTimeInterval: 0.1) }
                }
                let report = try self.rpc("status")
                DispatchQueue.main.async { self.busy = false; self.render(report) }
            } catch { DispatchQueue.main.async { self.busy = false; self.status.stringValue = error.localizedDescription } }
        }
    }
    @objc func run() { send("run") }
    @objc func cancel() { send("cancel") }
    @objc func stop() { send("stop") }
    func send(_ command: String) {
        guard !busy else {
            if command != "status" { DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) { self.send(command) } }
            return
        }
        busy = true
        worker.async {
            do {
                let report = try self.rpc(command)
                DispatchQueue.main.async { self.busy = false; self.render(report); if command == "stop" { self.stopped = true; self.status.stringValue = "Host 已安全停止 · 可再次启动恢复记录" } }
            } catch { DispatchQueue.main.async { self.busy = false; self.status.stringValue = error.localizedDescription } }
        }
    }
    func render(_ report: [String: Any]) {
        guard report["ok"] as? Bool == true else { status.stringValue = report["error"] as? String ?? "请求失败"; return }
        guard let d = report["diagnostics"] as? [String: Any], d["contract_version"] as? Int == 1 else { status.stringValue = "不支持的核心契约版本"; return }
        lastReport = d
        status.stringValue = "Host 运行中 · 模拟 · epoch \(d["epoch"] ?? "?") · 核心 \(d["core_version"] ?? "?")"
        if let error = report["host_error"] as? String { status.stringValue = "Host 错误：\(error)" }
        var lines = ["任务 / 会话 / 路由尝试 / 消息 / 事件（所有记录均为合成数据）", ""]
        for key in ["tasks", "sessions", "messages", "attempts", "events"] {
            lines.append("── \(key) ──")
            if let rows = d[key] as? [[String: Any]] {
                for row in rows { lines.append(row.keys.sorted().map { "\($0): \(row[$0]!)" }.joined(separator: "  ·  ")) }
            }
            lines.append("")
        }
        text.string = lines.joined(separator: "\n")
    }
    @objc func exportFeedback() {
        let panel = NSSavePanel(); panel.nameFieldStringValue = "velune-feedback.json"
        guard panel.runModal() == .OK, let destination = panel.url else { return }
        do {
            var report: [String: Any] = ["diagnostics": lastReport, "steps": "请填写复现步骤", "expected": "", "actual": "", "os_version": ProcessInfo.processInfo.operatingSystemVersionString]
            if let url = Bundle.main.url(forResource: "build-manifest", withExtension: "json"), let data = try? Data(contentsOf: url), let manifest = try? JSONSerialization.jsonObject(with: data) { report["build"] = manifest }
            let data = try JSONSerialization.data(withJSONObject: report, options: [.prettyPrinted, .sortedKeys])
            try data.write(to: destination, options: .atomic)
            status.stringValue = "反馈已本地导出，未上传；请补充步骤后自行分享。"
        } catch { status.stringValue = "导出失败：\(error.localizedDescription)" }
    }
}
let app = NSApplication.shared
let delegate = AppDelegate()
app.delegate = delegate
app.setActivationPolicy(.regular)
app.run()
