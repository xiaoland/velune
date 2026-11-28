#if VELUNE_DEV_CHECKS
import AppKit
import SwiftUI

@MainActor private final class SidebarCheckCanvas: NSView {
    override var isFlipped: Bool { true }
    override func draw(_ dirtyRect: NSRect) {
        NSColor.windowBackgroundColor.setFill()
        dirtyRect.fill()
    }
}

/// Header layout only: unwindowed hosts do not expose SwiftUI action labels
/// or materialize List rows. Action identity and behavior use the real-app AX check.
/// No scenes, panels, transport or input events.
@MainActor func sidebarActionsChecks(home: URL) {
    precondition(NSApp.windows.isEmpty && !NSApp.isActive, "离屏检查入口不能创建或激活窗口")
    let preferences = UserDefaults.standard.volatileDomain(forName: UserDefaults.argumentDomain)
    var syntheticPreferences = preferences
    syntheticPreferences["toolbarButtonsHidden"] = [String]()
    syntheticPreferences["toolbarButtonOrder"] = [String]()
    UserDefaults.standard.setVolatileDomain(syntheticPreferences, forName: UserDefaults.argumentDomain)
    defer { UserDefaults.standard.setVolatileDomain(preferences, forName: UserDefaults.argumentDomain) }

    func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
    var evidence: [[String: Any]] = []
    var failures: [String] = []
    func require(_ condition: Bool, _ message: String) {
        if !condition { failures.append(message) }
    }
    for dark in [false, true] {
        let appearance = NSAppearance(named: dark ? .darkAqua : .aqua)!
        NSApp.appearance = appearance
        for width: CGFloat in [240, 420] {
            let model = ConversationWindowModel(application: DeviceStore(preview: true), kind: .workspace)
            let content = DetachedToolContent(kind: .list, window: model)
            let size = NSSize(width: width, height: 500)
            let host = NSHostingView(rootView: content.applicationAppearance()
                .environment(\.colorScheme, dark ? .dark : .light)
                .frame(width: size.width, height: size.height))
            host.frame = NSRect(origin: .zero, size: size)
            host.appearance = appearance
            let canvas = SidebarCheckCanvas(frame: host.frame)
            canvas.appearance = appearance
            canvas.addSubview(host)
            host.layoutSubtreeIfNeeded()
            RunLoop.main.run(until: Date().addingTimeInterval(0.15))
            host.layoutSubtreeIfNeeded()
            precondition(host.window == nil && model.window == nil && NSApp.windows.isEmpty && !NSApp.isActive,
                         "生产列表离屏布局不得隐式创建窗口")
            let native = descendants(host)
            let searchFields = native.compactMap { $0 as? NSSearchField }.filter { !$0.isHiddenOrHasHiddenAncestor }
            let buttons = native.compactMap { $0 as? NSButton }.filter { !$0.isHiddenOrHasHiddenAncestor }
            func label(_ button: NSButton) -> String {
                [button.title, button.accessibilityLabel() ?? "", button.toolTip ?? ""].joined(separator: " | ")
            }
            func top(_ button: NSButton) -> CGFloat {
                let rect = host.convert(button.bounds, from: button)
                return host.isFlipped ? rect.minY : host.bounds.height - rect.maxY
            }
            let rowLimit: CGFloat = 52
            let headerButtons = buttons.filter { top($0) >= -1 && top($0) < rowLimit }
            let headerPopups = headerButtons.filter { $0 is NSPopUpButton }
            let headerPlainButtons = headerButtons.filter { !($0 is NSPopUpButton) }
            let records = buttons.map { button -> [String: Any] in
                let rect = host.convert(button.bounds, from: button)
                return ["type": String(describing: type(of: button)), "label": label(button),
                        "x": rect.minX, "top": host.isFlipped ? rect.minY : host.bounds.height - rect.maxY,
                        "width": rect.width, "height": rect.height]
            }
            let name = "sidebar-window-content-\(dark ? "dark" : "light")-\(Int(width))"
            let item: [String: Any] = ["case": name, "width": width, "searchFields": searchFields.count,
                "headerPopups": headerPopups.count, "headerPlainButtons": headerPlainButtons.count,
                "buttons": records, "windows": NSApp.windows.count, "scope": "header-layout-only",
                "actionLabels": "unavailable-without-window", "listRows": "not-materialized",
                "semanticEvidence": "real-app AX and interactions are verified separately"]
            evidence.append(item)
            // Save diagnostics before asserting so an unsupported native host
            // can be diagnosed without relaxing the expected action contract.
            try! JSONSerialization.data(withJSONObject: evidence, options: [.prettyPrinted, .sortedKeys])
                .write(to: home.appendingPathComponent("sidebar-check.json"))
            guard let bitmap = canvas.bitmapImageRepForCachingDisplay(in: canvas.bounds) else { fatalError("无法绘制列表顶部布局") }
            appearance.performAsCurrentDrawingAppearance { canvas.cacheDisplay(in: canvas.bounds, to: bitmap) }
            try! bitmap.representation(using: .png, properties: [:])!.write(to: home.appendingPathComponent(name + ".png"))
            require(headerPopups.count == 1 && headerPlainButtons.count == 1,
                    "顶部原生控件类型与数量：\(item)")
            require(searchFields.count == 1, "列表在任何位置都有且只有自己的搜索框：\(item)")
            for button in headerButtons {
                let rect = host.convert(button.bounds, from: button)
                require(rect.width > 0 && rect.height > 0 && rect.minX >= -1 && rect.maxX <= width + 1
                    && top(button) + rect.height <= rowLimit, "原生控件应在自己的顶部行内：\(item)")
            }
            model.list.close()
            model.focused.store.close()
        }
    }
    precondition(NSApp.windows.isEmpty && !NSApp.isActive)
    try! failures.joined(separator: "\n").write(to: home.appendingPathComponent("sidebar-failures.txt"), atomically: true, encoding: .utf8)
    precondition(failures.isEmpty, "列表离屏检查失败：\(failures.joined(separator: "\n"))")
    FileHandle.standardError.write(Data("PASS sidebar layout-only: 4 unwindowed detached list header captures (light/dark at 240/420); native control type counts, search field counts, top-row bounds; zero windows/models. Action AX labels and List rows are unavailable without windows; semantic identity and behavior require separate real-app evidence. Floating content is not a panel interaction.\n".utf8))
}
#endif
