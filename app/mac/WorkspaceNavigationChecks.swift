import AppKit
import Combine
import SwiftUI

/// Real window and pane models, using only synthetic preview conversations.
@MainActor func workspaceNavigationChecks(home: URL) {
    func note(_ text: String) { FileHandle.standardError.write(Data((text + "\n").utf8)) }
    workspaceKeyboardPaneFocusChecks()
    let device = DeviceStore(preview: true)
    func tab() -> ConversationTab { ConversationTab(store: ConversationStore(device: device), start: {}) }
    let tabs = [tab(), tab(), tab(), tab()]
    let pane = Pane(tabs: tabs, selected: tabs[2].id)
    pane.remove(tabs[2].id)
    precondition(pane.selectedID == tabs[1].id, "没有激活历史时关闭当前标签应选左邻")
    pane.select(tabs[3].id)
    pane.select(tabs[0].id)
    pane.remove(tabs[0].id)
    precondition(pane.selectedID == tabs[3].id, "关闭当前标签恢复最近激活者，不按左右位置")
    pane.remove(tabs[1].id)
    precondition(pane.selectedID == tabs[3].id, "关闭后台标签不改变当前激活者")
    note("PASS tab close: per-group activation history, left fallback, background close")

    let window = ConversationWindowModel(application: device, kind: .workspace)
    window.start(restoring: nil, seed: WindowSeed(conversationID: "sample-0"))
    let first = window.focused.id
    let originalTab = window.focused
    let ordinaryDraft = window.composer(for: originalTab)
    ordinaryDraft.restore("保留普通输入", attachments: [])
    guard let quickOpen = window.paletteItems(.conversations).first(where: { $0.id == "sample-1" }) else { fatalError("缺少合成会话候选") }
    quickOpen.run(false)
    precondition(window.tabCount == 2 && originalTab.store.loadedConversationID == "sample-0"
                 && ordinaryDraft.submission == "保留普通输入" && window.focused.store.device === device,
                 "快速打开默认新Tab并保留来源及输入和执行设备")
    quickOpen.run(true)
    precondition(window.tabCount == 2, "修饰键确认已有目标也复用同设备Tab")
    let middle = window.focused.id
    window.openInNewTab("sample-2")
    let last = window.focused.id
    window.selectTab(at: 1)
    precondition(window.focused.id == first, "序号从当前组左侧开始")
    window.selectTab(at: 9)
    precondition(window.focused.id == first, "不存在的序号不改选择")
    window.perform(.selectTab3)
    precondition(window.focused.id == last, "命令定位组内第3个标签")
    precondition(!window.canPerform(.selectTab9))
    window.openInNewTab("sample-0", device: device)
    precondition(window.tabCount == 3 && window.focused.id == first, "已打开目标复用标签")
    note("PASS indexed tab commands: left-to-right, missing slot no-op, open target reused")
    window.selectTab(first)
    window.closeTab(middle)
    window.selectTab(last)
    window.reopenClosedTab()
    precondition(window.focusedPane.tabs.map(\.id) == [first, middle, last], "关闭恢复应放回原索引并保留标签身份")
    precondition(window.focused.id == middle && window.focused.store.loadedConversationID == "sample-1")
    window.split(.vertical)
    let splitPane = window.focusedPaneID
    let blank = window.focused.id
    let splitLayout = window.layout
    window.selectTab(at: 1)
    precondition(window.focused.id == blank, "序号只作用于当前组")
    window.focusNeighbour(forward: false)
    precondition(window.focusedPaneID != splitPane, "上一个组切换聚焦")
    window.focusNeighbour(forward: true)
    precondition(window.focusedPaneID == splitPane, "下一个组恢复聚焦")
    window.closeTab(blank)
    precondition(!window.isSplit)
    window.reopenClosedTab()
    precondition(window.layout == splitLayout && window.focusedPaneID == splitPane && window.focused.id == blank,
                 "恢复最后一个组内标签时恢复组的位置和空标签身份")
    note("PASS closed tab recovery: original index, stable identity, removed split group")

    let lone = ConversationWindowModel(application: device, kind: .workspace)
    lone.start(restoring: nil, seed: WindowSeed(conversationID: "sample-0"))
    final class CloseWindow: NSWindow {
        var closeCount = 0
        override func close() { closeCount += 1 }
    }
    let native = CloseWindow(contentRect: NSRect(x: 0, y: 0, width: 600, height: 400), styleMask: [.titled], backing: .buffered, defer: false)
    native.isReleasedWhenClosed = false
    let anchor = NSView()
    native.contentView = anchor
    lone.register(lone.focusedPaneID, anchor: { anchor })
    lone.closeTab(lone.focused.id)
    precondition(native.closeCount == 0 && lone.tabCount == 1 && lone.focused.isBlank,
                 "关闭最后文档应留一个空白标签")
    lone.closeTab(lone.focused.id)
    precondition(native.closeCount == 1, "关闭最后空白标签才关闭窗口")
    note("PASS final tab: document → blank → window close")

    let handle = TabHandle.HandleView(frame: NSRect(x: 0, y: 0, width: 110, height: 28))
    anchor.addSubview(handle)
    handle.toolTip = "一个超出标签宽度的完整会话标题"
    let external = NSTrackingArea(rect: handle.bounds, options: [.mouseEnteredAndExited, .activeAlways], owner: native)
    handle.addTrackingArea(external)
    handle.updateTrackingAreas()
    handle.updateTrackingAreas()
    precondition(handle.trackingAreas.contains { $0 === external }, "更新 hover 不删除系统 tooltip 等其他 owner 的 tracking area")
    precondition(handle.trackingAreas.contains { $0 !== external && ($0.owner as AnyObject?) !== handle },
                 "原生 tooltip manager 的 tracking area 必须保留")
    precondition(handle.toolTip == "一个超出标签宽度的完整会话标题")
    note("PASS tab tooltip: full title and foreign tracking survive hover updates")

    precondition(ConversationListClicks.openDelay(pressedAt: 10, now: 10.1, interval: 0.5) > 0.39)
    precondition(ConversationListClicks.openDelay(pressedAt: 10, now: 10.6, interval: 0.5) == 0,
                 "原生 tracking 已等待完整双击间隔时不再追加等待")
    let start = ProcessInfo.processInfo.systemUptime
    for _ in 0..<20 {
        window.open("sample-0", device: device)
        window.open("sample-1", device: device)
    }
    let elapsed = ProcessInfo.processInfo.systemUptime - start
    precondition(window.focused.store.loadedConversationID == "sample-1")
    note(String(format: "PASS existing-tab navigation: 40 synthetic switches in %.3f s; double-click delay counted once from native event", elapsed))

    let aggregated = AggregatedListModel(local: device, hub: nil)
    var successes: [(String, Set<String>)] = []
    let watch = aggregated.managedConversations.sink { successes.append(($0.deviceID, $0.conversations)) }
    device.manageConversations(source: "合成成功", ids: ["sample-0"], operation: { true }, apply: { _ in })
    let deadline = Date().addingTimeInterval(3)
    while successes.isEmpty && Date() < deadline { RunLoop.main.run(until: Date().addingTimeInterval(0.01)) }
    precondition(successes.count == 1 && successes[0].0 == "local" && successes[0].1 == ["sample-0"])
    let selected: Set<AggregatedRowID> = [
        .init(section: "local‖all", conversation: "sample-0"),
        .init(section: "local‖all", conversation: "sample-1"),
        .init(section: "peer:synthetic‖all", conversation: "sample-0"),
    ]
    let remaining = selected.filter { !$0.belongs(to: successes[0].0, conversations: successes[0].1) }
    precondition(remaining.count == 2, "完成整理只清除同设备受处理项，保留其他选择")
    withExtendedLifetime(watch) {}
    note("PASS organization completion: successful, device-qualified items clear independently")
    conversationManagementCancellationChecks()
    conversationManagementVisualChecks(home: home)
}

/// Actual AppKit responders and production composer coordinators: model-only
/// group selection cannot prove where the next typed character will go.
@MainActor private func workspaceKeyboardPaneFocusChecks() {
    func wait(_ condition: () -> Bool) {
        let deadline = Date().addingTimeInterval(3)
        while !condition(), Date() < deadline { RunLoop.main.run(until: Date().addingTimeInterval(0.01)) }
        precondition(condition(), "等待原生组输入焦点超时")
    }
    let local = DeviceStore(preview: true)
    let peer = DeviceStore(preview: true, peerID: "synthetic-keyboard-peer")
    let workspace = ConversationWindowModel(application: local, kind: .workspace)
    workspace.tools.move(.composer, to: .builtIn)
    workspace.start(restoring: nil, seed: WindowSeed(conversationID: "sample-0"))
    let leftID = workspace.focusedPaneID, leftTab = workspace.focused
    workspace.split(.vertical)
    workspace.openInNewTab("sample-1", device: peer)
    let rightID = workspace.focusedPaneID, rightTab = workspace.focused
    let left = workspace.composer(for: leftTab), right = workspace.composer(for: rightTab)
    left.restoreSavedDraft(.init(text: "left draft", attachments: []))
    right.restoreSavedDraft(.init(text: "right draft", attachments: []))
    let native = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 900, height: 180), styleMask: [.borderless], backing: .buffered, defer: false)
    native.isReleasedWhenClosed = false
    let host = NSStackView()
    host.orientation = .horizontal
    host.distribution = .fillEqually
    native.contentView = host
    let leftHost = NSHostingView(rootView: ComposerEditor(model: left, placeholder: "", submit: {}, steer: {}, focused: { workspace.focusConversation(leftTab) }))
    let rightHost = NSHostingView(rootView: ComposerEditor(model: right, placeholder: "", submit: {}, steer: {}, focused: { workspace.focusConversation(rightTab) }))
    host.addArrangedSubview(leftHost); host.addArrangedSubview(rightHost)
    workspace.register(leftID, anchor: { leftHost })
    workspace.register(rightID, anchor: { rightHost })
    host.layoutSubtreeIfNeeded(); wait { left.textView != nil && right.textView != nil }
    guard let leftView = left.textView, let rightView = right.textView else { fatalError("合成双组原生输入框未挂载") }
    precondition(native.makeFirstResponder(rightView))
    workspace.focusNeighbour(forward: false)
    wait { native.firstResponder === leftView }
    precondition(workspace.focusedPaneID == leftID && native.firstResponder === leftView,
                 "上一个组必须同时转移AppKit输入焦点，不能只改变命令目标")
    leftView.setSelectedRange(NSRange(location: (leftView.string as NSString).length, length: 0))
    (native.firstResponder as! NSTextView).insertText(" typed-left", replacementRange: leftView.selectedRange())
    precondition(left.submission == "left draft typed-left" && right.submission == "right draft",
                 "切组后真实输入只改变目的设备会话的composer")
    workspace.focusNeighbour(forward: true)
    wait { native.firstResponder === rightView }
    precondition(workspace.focusedPaneID == rightID && native.firstResponder === rightView)
    rightView.setSelectedRange(NSRange(location: (rightView.string as NSString).length, length: 0))
    (native.firstResponder as! NSTextView).insertText(" typed-right", replacementRange: rightView.selectedRange())
    precondition(right.submission == "right draft typed-right" && left.submission == "left draft typed-left")
    workspace.focus(leftID)
    precondition(native.firstResponder === rightView, "鼠标驱动的组选择不能抢走所点控件的first responder")
    let target = rightHost.convert(NSPoint(x: rightHost.bounds.midX, y: rightHost.bounds.midY), to: nil)
    func press(_ type: NSEvent.EventType, modifiers: NSEvent.ModifierFlags = []) -> NSEvent {
        NSEvent.mouseEvent(with: type, location: target, modifierFlags: modifiers,
                           timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: native.windowNumber,
                           context: nil, eventNumber: 1, clickCount: 1, pressure: 1)!
    }
    for event in [press(.rightMouseDown), press(.leftMouseDown, modifiers: .control), press(.otherMouseDown)] {
        precondition(workspace.handleMouseDown(event) === event)
        precondition(workspace.focusedPaneID == leftID && workspace.focused === leftTab,
                     "打开后台组的上下文菜单或中键关闭不能激活该组")
        precondition(workspace.composer(for: leftTab) === left && workspace.composer(for: rightTab) === right
                     && left.submission == "left draft typed-left" && right.submission == "right draft typed-right",
                     "非激活操作不能切换或覆盖输入状态")
    }
    _ = workspace.handleMouseDown(press(.leftMouseDown))
    precondition(workspace.focusedPaneID == rightID, "同位置的普通左键仍聚焦目标组，避免检查仅因未命中而通过")
    workspace.focus(leftID)
    FileHandle.standardError.write(Data("PASS contextual pane press: right/control/middle preserve active tab and draft ownership; primary click still focuses target\n".utf8))
    let artifact = ArtifactTab(document: .mermaid(source: "flowchart LR; A-->B", conversationID: nil, sourceID: "keyboard-artifact"))
    workspace.focusedPane.insert(artifact)
    workspace.focus(rightID)
    workspace.focusNeighbour(forward: false)
    precondition(workspace.focusedEditor === artifact && native.firstResponder !== rightView,
                 "只读Artifact组不能继续把文字写到上一组的composer")
    workspace.register(leftID, anchor: { nil })
    workspace.register(rightID, anchor: { nil })
    native.contentView = nil
    FileHandle.standardError.write(Data("PASS keyboard pane focus: native first responder and inserted text follow destination; peer input remains independent; pointer focus unchanged; Artifact clears stale editor\n".utf8))
}

@MainActor private func conversationManagementVisualChecks(home: URL) {
    let local = DeviceStore(preview: true)
    let peer = DeviceStore(preview: true, peerID: "synthetic-queue-peer")
    let aggregated = AggregatedListModel(local: local, hub: nil)
    aggregated.updateDevices([
        .init(peerID: nil, store: local, name: "本机合成设备"),
        .init(peerID: "synthetic-queue-peer", store: peer, name: "远端合成设备"),
    ])
    let localStarted = DispatchSemaphore(value: 0), releaseLocal = DispatchSemaphore(value: 0)
    let peerStarted = DispatchSemaphore(value: 0), releasePeer = DispatchSemaphore(value: 0)
    local.manageConversations(source: "重命名会话", ids: ["running"], operation: {
        localStarted.signal(); _ = releaseLocal.wait(timeout: .now() + 8); return true
    }, apply: { _ in })
    peer.performOnQueue { peerStarted.signal(); _ = releasePeer.wait(timeout: .now() + 8) }
    precondition(localStarted.wait(timeout: .now() + 3) == .success)
    precondition(peerStarted.wait(timeout: .now() + 3) == .success)
    peer.manageConversations(source: "归档会话", ids: ["waiting"], operation: { () -> Bool in
        preconditionFailure("通过原生取消按钮取消的排队项不能执行")
    }, apply: { _ in })
    let activeID = local.conversationManagement[0].id, waitingID = peer.conversationManagement[0].id
    let content = ConversationOperationDetails(aggregated: aggregated).applicationAppearance()
        .frame(width: 420, height: 140).background(Color(nsColor: .windowBackgroundColor))
    let view = NSHostingView(rootView: content)
    let native = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 420, height: 140), styleMask: [.borderless], backing: .buffered, defer: false)
    native.isReleasedWhenClosed = false
    native.contentView = view
    view.layoutSubtreeIfNeeded()
    RunLoop.main.run(until: Date().addingTimeInterval(0.2))
    view.layoutSubtreeIfNeeded(); view.displayIfNeeded()
    guard let bitmap = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { fatalError("无法绘制整理队列") }
    view.cacheDisplay(in: view.bounds, to: bitmap)
    try! bitmap.representation(using: .png, properties: [:])!.write(to: home.appendingPathComponent("management-queue.png"))
    func button(_ node: Any, id: String) -> (enabled: Bool, press: () -> Bool)? {
        var visited = Set<ObjectIdentifier>()
        func find(_ node: Any) -> (enabled: Bool, press: () -> Bool)? {
            guard let object = node as? NSObject, visited.insert(ObjectIdentifier(object)).inserted else { return nil }
            if let element = node as? NSAccessibilityProtocol {
                if element.accessibilityIdentifier() == id { return (element.isAccessibilityEnabled(), { element.accessibilityPerformPress() }) }
                for child in element.accessibilityChildren() ?? [] { if let found = find(child) { return found } }
            }
            if let view = node as? NSView {
                for child in view.subviews { if let found = find(child) { return found } }
            }
            return nil
        }
        return find(node)
    }
    if button(view, id: "cancel-management-\(activeID)") != nil { fatalError("执行项不能显示取消按钮") }
    func descendants(_ node: NSView) -> [NSView] { [node] + node.subviews.flatMap(descendants) }
    // SwiftUI draws this label in a child view; NSButton.title is empty.
    // This fixture has exactly one waiting row, hence one native button.
    let nativeButtons = descendants(view).compactMap { $0 as? NSButton }
    let nativeCancel: (enabled: Bool, press: () -> Bool)? = nativeButtons.count == 1 ? nativeButtons.first.map { control in
        (control.isEnabled, { control.performClick(nil); return true })
    } : nil
    guard let cancel = button(view, id: "cancel-management-\(waitingID)") ?? nativeCancel else {
        for node in descendants(view) {
            FileHandle.standardError.write(Data("UI node \(type(of: node)) id=\(node.accessibilityIdentifier()) label=\(String(describing: node.accessibilityLabel()))\n".utf8))
        }
        fatalError("等待项缺少可操作的原生取消按钮")
    }
    precondition(cancel.enabled && cancel.press(), "等待项原生取消按钮必须可操作")
    precondition(peer.conversationManagement.isEmpty, "原生按钮必须取消对应远端设备队列")
    precondition(!native.isVisible && !native.isKeyWindow && !NSApp.isActive)
    releasePeer.signal(); releaseLocal.signal()
    RunLoop.main.run(until: Date().addingTimeInterval(0.1))
    native.contentView = nil
    FileHandle.standardError.write(Data("PASS management native UI: waiting cancel enabled and actionable, running cancel absent, devices explicit, hidden rendered image\n".utf8))
}

@MainActor private func conversationManagementCancellationChecks() {
    func wait(_ ready: () -> Bool) {
        let deadline = Date().addingTimeInterval(3)
        while !ready() && Date() < deadline { RunLoop.main.run(until: Date().addingTimeInterval(0.01)) }
        precondition(ready(), "整理操作合成队列超时")
    }
    final class Effects: @unchecked Sendable {
        let lock = NSLock()
        private var performed: [String] = []
        func append(_ value: String) { lock.withLock { performed.append(value) } }
        var values: [String] { lock.withLock { performed } }
    }
    let device = DeviceStore(preview: true)
    let effects = Effects()
    let started = DispatchSemaphore(value: 0)
    let release = DispatchSemaphore(value: 0)
    var successes: [Set<String>] = []
    let watch = device.managedConversations.sink { successes.append($0) }
    device.manageConversations(source: "正在执行", ids: ["active"], operation: {
        started.signal()
        _ = release.wait(timeout: .now() + 3)
        effects.append("active")
        return true
    }, apply: { _ in })
    // Pause main-loop delivery so UI still says waiting while the native
    // operation has already crossed its admission boundary.
    precondition(started.wait(timeout: .now() + 3) == .success)
    let active = device.conversationManagement[0].id
    precondition(!device.cancelConversationManagement(active), "已执行但UI回报尚未到达时不能假取消")
    precondition(device.conversationManagement[0].running)
    device.manageConversations(source: "将取消", ids: ["cancelled"], operation: {
        effects.append("cancelled")
        return true
    }, apply: { _ in preconditionFailure("取消项不得应用结果") })
    let cancelled = device.conversationManagement.last!.id
    precondition(device.cancelConversationManagement(cancelled))
    precondition(!device.cancelConversationManagement(cancelled))
    device.manageConversations(source: "后续项", ids: ["next"], operation: {
        effects.append("next")
        return true
    }, apply: { _ in })
    release.signal()
    wait { device.conversationManagement.isEmpty }
    precondition(effects.values == ["active", "next"], "取消等待项不得执行原生修改，也不得阻止后续操作")
    precondition(successes == [["active"], ["next"]], "取消不发送完成事件，列表保留取消项选择")
    device.manageConversations(source: "部分删除", ids: ["deleted", "failed"], deleting: true,
                               operation: { Set(["deleted"]) }, completedIDs: { $0 }, apply: { _ in })
    wait { device.conversationManagement.isEmpty }
    precondition(successes.last == ["deleted"], "部分删除成功不能清除失败项选择")
    withExtendedLifetime(watch) {}
    FileHandle.standardError.write(Data("PASS management cancellation: waiting effects skipped, active race rejected, later work proceeds, cancelled selection retained\n".utf8))
}
