#!/usr/bin/env python3
"""Explicit native transcript scroll/geometry diagnostic; synthetic content only, never CI.

Compiles the actual Mac transcript sources into a temporary probe, loads a long
synthetic history into a window (shown only with --snapshot, which captures that
window alone) and checks: no visible displacement
of the reading row while scrolling fast through unmeasured history, displayed
text geometry equals the measured layout, far jumps, bottom following, width
changes, local disclosure toggles, the in-place outline collapse/expand (animated
only in a visible window) and that parked reusable cells stay hidden. Prints
timings; exits non-zero on failure.
"""
import argparse
import os
import shutil
import subprocess
import tempfile
from pathlib import Path

MAIN = r'''
import SwiftUI
import AppKit
@MainActor func spin(_ count: Int = 1) async { for _ in 0..<count { await withCheckedContinuation { c in DispatchQueue.main.async { c.resume() } } } }
@MainActor func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
@MainActor func fail(_ message: String) -> Never { FileHandle.standardError.write(Data(("FAIL " + message + "\n").utf8)); exit(1) }
@MainActor func note(_ message: String) { print(message); fflush(stdout) }
@MainActor final class Driver: ObservableObject {
    @Published var request: UInt64 = 0; @Published var jump: TranscriptJump?; var following = true; var payloadRequests: [String] = []
    @Published var collapsed = false; @Published var anchor: String?
}
/// Mirrors TranscriptView's inputs, adding explicit jump requests.
struct Harness: View {
    @ObservedObject var model: TranscriptModel
    @ObservedObject var driver: Driver
    var body: some View {
        let userIDs = Set(model.userRows.map(\.id))
        let rows = driver.collapsed ? model.rows.filter { userIDs.contains($0.id) } : model.rows
        TranscriptTable(conversationID: "probe", items: model.items(for: rows), revision: model.contentRevision, activity: nil,
                        outlineCollapsed: driver.collapsed, outlineAnchorID: driver.anchor, cwd: "/tmp", bottomRequest: driver.request, jump: driver.jump,
                        followingChanged: { driver.following = $0 }, outlineToggle: { driver.anchor = $0; driver.collapsed = false }, loadPayloads: { driver.payloadRequests += $0 })
    }
}
func synthetic(_ count: Int) -> [Message] {
    var messages: [Message] = []
    let paragraph = String(repeating: "合成段落文字 **强调**、`代码`与[链接](https://example.invalid)，用于稳定宽度排版检查。 ", count: 5)
    for i in 0..<count {
        messages.append(Message(id: "u\(i)", role: .user, blocks: [.text("合成用户问题 \(i)" + (i % 3 == 0 ? "\n第二行补充说明，较长的用户输入需要在气泡中换行显示。" : ""))]))
        var body = "## 合成回复 \(i)\n\n" + String(repeating: paragraph + "\n\n", count: i % 5 == 0 ? 8 : 2)
        body += "- 列表项一\n- 列表项二\n  1. 嵌套有序\n  2. 第二项\n\n> 引用内容\n\n"
        body += "```swift\nfunc probe\(i)() -> Int {\n    return \(i)\n}\n```\n\n| 列 | 值 |\n| --- | ---: |\n| 合成 | \(i) |\n"
        // Every fourth tool output is large and arrives by size only, as from the core.
        let output: MessagePayload = i % 4 == 0 ? .deferred(bytes: 30_000) : .inline("输出\n第二行")
        messages.append(Message(id: "a\(i)", role: .assistant, blocks: [.reasoning(.inline("推理 \(i)")), .text(body), .tool(id: "t\(i)", title: "读取文件 \(i)", state: .completed, output: output)], stamp: 1))
    }
    return messages
}
@MainActor func apply(_ model: TranscriptModel, _ messages: [Message]) {
    model.apply(messages, turns: [], items: messages.map { .message(id: $0.id, messageID: $0.id) },
                outline: messages.filter { $0.role == .user }.map { .init(id: $0.id, messageID: $0.id) },
                identities: messages.map { .init(id: $0.id, messageID: $0.id) })
}
/// Materialized, non-hidden row cells in document order.
@MainActor func cells(_ document: TranscriptDocumentView) -> [TranscriptCellView] {
    document.subviews.compactMap { $0 as? TranscriptCellView }.filter { !$0.isHidden && $0.rowID != nil }.sorted { $0.frame.minY < $1.frame.minY }
}
@MainActor func cell(_ document: TranscriptDocumentView, _ id: String) -> TranscriptCellView? { cells(document).first { $0.rowID == id } }
@MainActor func top(_ document: TranscriptDocumentView, _ scroll: NSScrollView, _ id: String) -> CGFloat {
    guard let cell = cell(document, id) else { fail("row \(id) not materialized") }
    return cell.frame.minY - scroll.contentView.bounds.minY
}
@MainActor func rowAt(_ document: TranscriptDocumentView, _ y: CGFloat) -> String {
    guard let id = cells(document).first(where: { $0.frame.minY <= y && $0.frame.maxY > y })?.rowID else { fail("no row at \(y)") }
    return id
}
@MainActor func materialize(_ window: NSWindow) { window.contentView?.layoutSubtreeIfNeeded() }
@MainActor func gapToBottom(_ document: TranscriptDocumentView, _ scroll: NSScrollView) -> CGFloat { document.frame.maxY - scroll.contentView.bounds.maxY }
@MainActor func snapshot(_ scroll: NSScrollView, _ name: String) {
    guard let directory = ProcessInfo.processInfo.environment["PROBE_SNAPSHOT"] else { return }
    if let window = scroll.window, window.isVisible {
        window.displayIfNeeded()
        let task = Process(); task.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
        task.arguments = ["-x", "-o", "-l", String(window.windowNumber), directory + "/" + name + "-screen.png"]
        try? task.run(); task.waitUntilExit()
    }
    guard let rep = scroll.bitmapImageRepForCachingDisplay(in: scroll.bounds) else { return }
    scroll.cacheDisplay(in: scroll.bounds, to: rep)
    try? rep.representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: directory).appendingPathComponent(name + ".png"))
}
@MainActor func checkGeometry(_ document: TranscriptDocumentView, _ scroll: NSScrollView, _ label: String) {
    let clip = scroll.contentView.bounds
    if abs(document.frame.width - clip.width) > 0.5 { fail("\(label): document \(document.frame.width) != viewport \(clip.width)") }
    let shown = cells(document)
    // A parked reusable cell must stay hidden, or it draws stale content over live rows.
    if let parked = document.subviews.first(where: { ($0 as? TranscriptCellView).map { !$0.isHidden && $0.rowID == nil } ?? false }) { fail("\(label): unassigned cell visible at \(parked.frame)") }
    guard let first = shown.first, let last = shown.last, first.frame.minY <= clip.minY + 0.5 || first.frame.minY <= 0.5, last.frame.maxY >= min(clip.maxY, document.frame.maxY) - 0.5 else { fail("\(label): viewport not covered by cells") }
    var checked = 0
    for (index, cell) in shown.enumerated() {
        if index > 0, abs(shown[index - 1].frame.maxY - cell.frame.minY) > 0.01 { fail("\(label): gap/overlap before \(cell.rowID!)") }
        for view in descendants(cell).dropFirst() where !view.isHiddenOrHasHiddenAncestor && view.superview === cell {
            if view.frame.maxY > cell.bounds.height + 0.5 { fail("\(label): row \(cell.rowID!) \(type(of: view)) overflows cell \(view.frame.maxY) > \(cell.bounds.height)") }
        }
        for case let text as TranscriptTextView in descendants(cell) where !text.isHiddenOrHasHiddenAncestor {
            guard let lm = text.layoutManager, let tc = text.textContainer else { continue }
            lm.ensureLayout(for: tc)
            // Displayed TextKit layout must equal the measured frame (ceil of used height).
            let used = lm.usedRect(for: tc).height
            if used > text.frame.height + 0.01 || text.frame.height - used >= 1 { fail("\(label): row \(cell.rowID!) text used \(used) != frame \(text.frame.height)") }
            if text.convert(text.bounds, to: cell).maxY > cell.bounds.height + 0.5 { fail("\(label): row \(cell.rowID!) text overflows cell") }
            checked += 1
        }
    }
    note("GEOMETRY \(label) rows=\(shown.count) texts=\(checked) ok")
}
@main struct Probe {
    @MainActor static func main() async {
        let app = NSApplication.shared; app.setActivationPolicy(.prohibited)
        let model = TranscriptModel(); let driver = Driver()
        let count = Int(ProcessInfo.processInfo.environment["PROBE_PAIRS"] ?? "400")!
        var messages = synthetic(count)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 900, height: 700), styleMask: [.titled, .resizable], backing: .buffered, defer: false)
        window.contentView = NSHostingView(rootView: Harness(model: model, driver: driver))
        if ProcessInfo.processInfo.environment["PROBE_VISIBLE"] != nil { app.setActivationPolicy(.accessory); window.orderFrontRegardless() }
        await spin(3)
        let clock = ContinuousClock()
        let loadStart = clock.now
        apply(model, messages)
        await spin(2)
        guard let document = descendants(window.contentView!).compactMap({ $0 as? TranscriptDocumentView }).first, let scroll = document.enclosingScrollView else { fail("transcript absent") }
        materialize(window)
        note("LOAD rows=\(messages.count) firstPaint=\(clock.now - loadStart)")
        let clip = scroll.contentView
        if gapToBottom(document, scroll) > 1 { fail("not at bottom after load gap=\(gapToBottom(document, scroll))") }
        checkGeometry(document, scroll, "bottom")

        // Jump into unmeasured history immediately after load.
        driver.jump = TranscriptJump(id: "u7", token: 100); await spin(2); materialize(window)
        for _ in 0..<40 { await spin(); try? await Task.sleep(for: .milliseconds(2)) }
        if abs(top(document, scroll, "u7") + TranscriptRowLayout.rowInset - 8) > 1 { fail("early jump drifted to \(top(document, scroll, "u7"))") }
        checkGeometry(document, scroll, "early-jump")
        snapshot(scroll, "early-jump")
        driver.request &+= 1; await spin(3); materialize(window)
        if gapToBottom(document, scroll) > 1 { fail("bottom request after jump") }
        note("EARLY-JUMP ok")

        // Fast upward scroll immediately, while distant rows are still estimates.
        var maxJitter: CGFloat = 0; var worstStep = Duration.zero; var steps = 0
        let stepSize: CGFloat = 260
        while clip.bounds.minY > 0 && steps < 4000 {
            let id = rowAt(document, clip.bounds.midY)
            let before = top(document, scroll, id)
            let start = clock.now
            let delta = min(stepSize, clip.bounds.minY)
            clip.setBoundsOrigin(NSPoint(x: 0, y: clip.bounds.minY - delta))
            scroll.reflectScrolledClipView(clip)
            materialize(window)
            await spin()
            let elapsed = clock.now - start
            worstStep = max(worstStep, elapsed)
            if elapsed > .milliseconds(40) { note("SLOW step=\(steps) \(elapsed) row=\(id)") }
            let jitter = abs(top(document, scroll, id) - (before + delta))
            if jitter > 1 && maxJitter <= 1 { note("JITTER step=\(steps) row=\(id) before=\(before) after=\(top(document, scroll, id)) delta=\(delta)") }
            maxJitter = max(maxJitter, jitter)
            steps += 1
        }
        note("FASTSCROLL steps=\(steps) maxJitter=\(maxJitter) worstStep=\(worstStep)")
        if maxJitter > 1 { fail("reading row moved by \(maxJitter)pt beyond the scroll delta") }
        checkGeometry(document, scroll, "top")

        // Let background measurement finish, then check stability at a mid anchor.
        let settleStart = clock.now
        var lastHeight = document.frame.height; var stable = 0
        while stable < 30 { await spin(); try? await Task.sleep(for: .milliseconds(5)); if document.frame.height == lastHeight { stable += 1 } else { stable = 0; lastHeight = document.frame.height } }
        note("SETTLED documentHeight=\(document.frame.height) after=\(clock.now - settleStart)")
        clip.setBoundsOrigin(NSPoint(x: 0, y: document.frame.height / 2)); scroll.reflectScrolledClipView(clip)
        materialize(window); await spin(3)
        let midRow = rowAt(document, clip.bounds.minY + 10); let midTop = top(document, scroll, midRow)
        await spin(10)
        if abs(top(document, scroll, midRow) - midTop) > 0.5 { fail("settled history still moves") }
        checkGeometry(document, scroll, "middle")

        // Width change keeps the reading row.
        window.setContentSize(NSSize(width: 640, height: 700)); materialize(window); await spin(3)
        if abs(top(document, scroll, midRow) - midTop) > 1 { fail("narrowing moved reading row \(midTop) -> \(top(document, scroll, midRow))") }
        checkGeometry(document, scroll, "narrow")
        window.setContentSize(NSSize(width: 1200, height: 700)); materialize(window); await spin(3)
        if abs(top(document, scroll, midRow) - midTop) > 1 { fail("widening moved reading row") }
        checkGeometry(document, scroll, "wide")
        note("RESIZE ok")

        // Tail growth while reading history does not move the reading row.
        messages[messages.count - 1].blocks.append(.text(String(repeating: "尾部增长。", count: 400)))
        apply(model, messages); await spin(3); materialize(window)
        if abs(top(document, scroll, midRow) - midTop) > 1 { fail("tail growth moved reading row") }
        note("TAIL-WHILE-READING ok")

        // Local disclosure: the toggled row keeps its place.
        guard let toolCell = cells(document).first(where: { $0.frame.minY >= clip.bounds.minY && descendants($0).contains { ($0 as? NSButton)?.bezelStyle == .disclosure && $0.superview?.isHidden == false } }),
              let button = descendants(toolCell).compactMap({ $0 as? NSButton }).last(where: { $0.bezelStyle == .disclosure && $0.superview?.isHidden == false }), let toolID = toolCell.rowID else { fail("no visible disclosure") }
        let toolHeight = toolCell.frame.height; let toolTop = top(document, scroll, toolID)
        button.performClick(nil); await spin(3); materialize(window)
        guard let grown = cell(document, toolID), grown.frame.height > toolHeight else { fail("disclosure did not grow row") }
        if abs(top(document, scroll, toolID) - toolTop) > 1 { fail("disclosure moved its row") }
        checkGeometry(document, scroll, "expanded")
        snapshot(scroll, "expanded")
        note("DISCLOSURE ok \(toolHeight) -> \(grown.frame.height)")

        // A deferred tool output is requested when expanded and fills in place.
        driver.jump = TranscriptJump(id: "a148", token: 40); await spin(3); materialize(window); await spin(3)
        guard let deferredCell = cell(document, "a148"),
              let deferredButton = descendants(deferredCell).compactMap({ $0 as? NSButton }).last(where: { $0.bezelStyle == .disclosure && $0.superview?.isHidden == false }) else { fail("deferred disclosure missing") }
        driver.payloadRequests = []
        deferredButton.performClick(nil); await spin(3); materialize(window)
        if !driver.payloadRequests.contains("a148") { fail("expanding deferred output requested \(driver.payloadRequests)") }
        driver.payloadRequests = []
        let placeholderHeight = cell(document, "a148")!.frame.height; let deferredTop = top(document, scroll, "a148")
        let index = messages.firstIndex { $0.id == "a148" }!
        messages[index].blocks[2] = .tool(id: "t148", title: "读取文件 148", state: .completed, output: .inline((0..<200).map { "第 \($0) 行合成输出" }.joined(separator: "\n")))
        apply(model, messages); await spin(3); materialize(window)
        guard let loaded = cell(document, "a148"), loaded.frame.height > placeholderHeight + 1000 else { fail("loaded output did not fill the row") }
        if abs(top(document, scroll, "a148") - deferredTop) > 1 { fail("loaded output moved its row") }
        if driver.payloadRequests.contains("a148") { fail("inline output was requested again") }
        checkGeometry(document, scroll, "payload-loaded")
        note("PAYLOAD ok \(placeholderHeight) -> \(loaded.frame.height)")

        // Far jumps land the target's top just below the viewport top, also after
        // tail growth and while history is only partly measured.
        for (token, id) in ["u20", "a150", "u390", "u3"].enumerated() {
            driver.jump = TranscriptJump(id: id, token: UInt64(token + 1)); await spin(3); materialize(window); await spin(3)
            if abs(top(document, scroll, id) + TranscriptRowLayout.rowInset - 8) > 1 { fail("jump \(id) top=\(top(document, scroll, id))") }
            if driver.following { fail("jump kept following") }
        }
        note("JUMP ok")

        // Whole-message copy is offered from a message's native text menu.
        driver.jump = TranscriptJump(id: "a150", token: 50); await spin(3); materialize(window); await spin(3)
        if let message = cell(document, "a150"),
           let text = descendants(message).compactMap({ $0 as? TranscriptTextView }).first(where: { !$0.isHiddenOrHasHiddenAncestor }),
           let menu = text.delegate?.textView?(text, menu: NSMenu(), for: NSEvent.mouseEvent(with: .rightMouseDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: 0, context: nil, eventNumber: 0, clickCount: 1, pressure: 1)!, at: 0),
           let item = menu.items.first, item.title == "拷贝整条消息" {
            // Restore the user's clipboard text after checking what was copied.
            let saved = NSPasteboard.general.string(forType: .string)
            NSApp.sendAction(item.action!, to: item.target, from: item)
            let copied = NSPasteboard.general.string(forType: .string)
            NSPasteboard.general.clearContents()
            if let saved { NSPasteboard.general.setString(saved, forType: .string) }
            guard copied?.hasPrefix("## 合成回复 150") == true else { fail("copied message source mismatch") }
            note("COPY-MESSAGE ok")
        } else { fail("message menu item missing") }

        // Back to bottom, then streaming growth follows.
        driver.request &+= 1; await spin(4); materialize(window)
        for i in 0..<5 {
            messages[messages.count - 1].blocks.append(.text("流式片段 \(i) " + String(repeating: "内容", count: 80)))
            apply(model, messages); await spin(3); materialize(window)
            if gapToBottom(document, scroll) > 1 { fail("following lost bottom gap=\(gapToBottom(document, scroll))") }
        }
        checkGeometry(document, scroll, "stream")
        note("FOLLOW ok")

        // A folded work turn expands into a separate member row and collapses back.
        let turn = TranscriptTurn(id: "turn0", userMessageID: "u0", workMessageIDs: ["a0"], lastMessageID: nil, durationMs: 1500, isRunning: false)
        var items: [TranscriptItem] = messages.map { .message(id: $0.id, messageID: $0.id) }
        items[1] = .work(turnID: "turn0")
        model.apply(messages, turns: [turn], items: items, outline: messages.filter { $0.role == .user }.map { .init(id: $0.id, messageID: $0.id) }, identities: messages.map { .init(id: $0.id, messageID: $0.id) })
        await spin(3)
        driver.jump = TranscriptJump(id: "u0", token: 60); await spin(3); materialize(window); await spin(2)
        guard let work = cell(document, "work:turn0"),
              let triangle = descendants(work).compactMap({ $0 as? NSButton }).first(where: { $0.bezelStyle == .disclosure && $0.superview?.isHidden == false }) else { fail("work header missing") }
        let workTop = top(document, scroll, "work:turn0")
        triangle.performClick(nil); await spin(3); materialize(window)
        if cell(document, "a0") == nil { fail("work expansion did not show its member") }
        if abs(top(document, scroll, "work:turn0") - workTop) > 1 { fail("work expansion moved its header \(workTop) -> \(top(document, scroll, "work:turn0"))") }
        checkGeometry(document, scroll, "work-expanded")
        triangle.performClick(nil); await spin(3); materialize(window)
        if cell(document, "a0") != nil { fail("work collapse kept its member") }
        note("WORK ok")

        // Tool rows inside an expanded work turn: expanding one pushes its siblings down.
        let shapes: [String] = [
            (0..<30).map { "工具 0 输出第 \($0) 行" }.joined(separator: "\n"),
            "",
            (0..<40).map { "/Users/example/very/long/path/\(String(repeating: "segment", count: 20))/file\($0).swift:\($0): " + String(repeating: "word ", count: 60) }.joined(separator: "\n"),
            (0..<40).map { "\t\tcol\t\($0)\r\u{1B}[32mgreen\u{1B}[0m " + String(repeating: "x", count: 400) }.joined(separator: "\n"),
            (0..<3000).map { "第 \($0) 行 😀 混合 mixed text" }.joined(separator: "\n"),
            String(repeating: "{\"key\":\"value\",\"n\":12345}", count: 400),
        ]
        let tools = (0..<6).map { k in Message(id: "w\(k)", role: .assistant, blocks: [.tool(id: "wt\(k)", title: "工具 \(k)", state: .completed, output: k == 1 ? .deferred(bytes: 40_000) : .inline(shapes[k]))], stamp: 1) }
        var workMessages = [messages[0]] + tools + [messages[1]]
        let toolTurn = TranscriptTurn(id: "turnw", userMessageID: "u0", workMessageIDs: tools.map(\.id), lastMessageID: "a0", durationMs: 900, isRunning: false)
        func applyWork() {
            let rest = Array(messages.dropFirst(2))
            let all = workMessages + rest
            var workItems: [TranscriptItem] = [.message(id: "u0", messageID: "u0"), .work(turnID: "turnw"), .message(id: "a0", messageID: "a0")]
            workItems += rest.map { .message(id: $0.id, messageID: $0.id) }
            model.apply(all, turns: [toolTurn], items: workItems, outline: all.filter { $0.role == .user }.map { .init(id: $0.id, messageID: $0.id) }, identities: all.map { .init(id: $0.id, messageID: $0.id) })
        }
        applyWork(); await spin(3)
        driver.jump = TranscriptJump(id: "u0", token: 70); await spin(3); materialize(window); await spin(2)
        guard let toolWork = cell(document, "work:turnw"),
              let toolTriangle = descendants(toolWork).compactMap({ $0 as? NSButton }).first(where: { $0.bezelStyle == .disclosure && $0.superview?.isHidden == false }) else { fail("tool work header missing") }
        toolTriangle.performClick(nil); await spin(3); materialize(window)
        checkGeometry(document, scroll, "tool-work-expanded")
        for k in [0, 1, 2, 3, 4] {
            if let row = cell(document, "w\(k)") { clip.scroll(to: NSPoint(x: 0, y: max(0, row.frame.minY - 150))); scroll.reflectScrolledClipView(clip); await spin(3); materialize(window) }
            guard let row = cell(document, "w\(k)"), let button = descendants(row).compactMap({ $0 as? NSButton }).first(where: { $0.bezelStyle == .disclosure && $0.superview?.isHidden == false }) else { fail("tool row w\(k) missing") }
            let nextTop = cell(document, "w\(k + 1)").map { $0.frame.minY }
            button.performClick(nil); await spin(3); materialize(window)
            checkGeometry(document, scroll, "tool-\(k)-expanded"); snapshot(scroll, "tool-\(k)")
            if let nextTop, let next = cell(document, "w\(k + 1)"), next.frame.minY <= nextTop { fail("expanding w\(k) did not push w\(k + 1)") }
            if k >= 2 { button.performClick(nil); await spin(3); materialize(window); checkGeometry(document, scroll, "tool-\(k)-collapsed") }
        }
        // The deferred output loads later, as from the Store.
        workMessages[2].blocks[0] = .tool(id: "wt1", title: "工具 1", state: .completed, output: .inline((0..<120).map { "载入输出第 \($0) 行" }.joined(separator: "\n")))
        applyWork(); await spin(3); materialize(window)
        checkGeometry(document, scroll, "tool-payload-loaded")
        // A later message update while tools stay expanded.
        workMessages[4].blocks[0] = .tool(id: "wt3", title: "工具 3", state: .completed, output: .inline((0..<90).map { "更新输出第 \($0) 行" }.joined(separator: "\n")))
        applyWork(); await spin(3); materialize(window)
        checkGeometry(document, scroll, "tool-updated")
        for _ in 0..<20 { await spin(); try? await Task.sleep(for: .milliseconds(5)) }
        checkGeometry(document, scroll, "tool-settled")
        note("TOOL-SIBLINGS ok")

        // In-place outline: collapsing keeps the section's user message in place
        // (at the top when it started above), shows only user rows, and a bubble
        // click expands back with the clicked row fixed. Visible windows animate.
        let animated = window.isVisible
        func settle() async { for _ in 0..<50 { await spin(); try? await Task.sleep(for: .milliseconds(10)) } ; materialize(window) }
        driver.jump = TranscriptJump(id: "u50", token: 80); await spin(3); materialize(window); await spin(3)
        let sectionTop = top(document, scroll, "u50")
        driver.anchor = nil; driver.collapsed = true; await spin(2)
        if animated {
            try? await Task.sleep(for: .milliseconds(120)); await spin()
            let fading = document.subviews.compactMap { $0 as? TranscriptCellView }.filter { !$0.isHidden && $0.alphaValue < 0.99 }
            if fading.isEmpty { fail("collapse showed no transition") }
            snapshot(scroll, "outline-collapsing")
        }
        await settle()
        let collapsedRows = cells(document).compactMap(\.rowID)
        if let stray = collapsedRows.first(where: { !$0.hasPrefix("u") && $0 != "transcript-end" }) { fail("collapsed list shows \(stray)") }
        if abs(top(document, scroll, "u50") - max(sectionTop, 0)) > 1 { fail("collapse moved anchor \(sectionTop) -> \(top(document, scroll, "u50"))") }
        if document.subviews.contains(where: { ($0 as? TranscriptCellView).map { !$0.isHidden && ($0.alphaValue < 1 || $0.clipsToBounds) } ?? false }) { fail("transition state left on cells") }
        checkGeometry(document, scroll, "outline-collapsed"); snapshot(scroll, "outline-collapsed")
        note("OUTLINE-COLLAPSE ok rows=\(collapsedRows.count)")

        guard let clicked = cells(document).first(where: { $0.rowID?.hasPrefix("u") == true && $0.frame.minY >= clip.bounds.minY + 200 }), let clickedID = clicked.rowID else { fail("no visible user row to expand") }
        let clickedTop = top(document, scroll, clickedID)
        clicked.perform(Selector(("bubbleClicked")))
        if driver.collapsed || driver.anchor != clickedID { fail("bubble click did not request expansion") }
        await spin(2)
        if animated { try? await Task.sleep(for: .milliseconds(120)); await spin(); snapshot(scroll, "outline-expanding") }
        await settle()
        if abs(top(document, scroll, clickedID) - clickedTop) > 1 { fail("expansion moved clicked row \(clickedTop) -> \(top(document, scroll, clickedID))") }
        let assistant = "a" + clickedID.dropFirst()
        if cell(document, assistant) == nil { fail("expanded list lacks \(assistant)") }
        checkGeometry(document, scroll, "outline-expanded"); snapshot(scroll, "outline-expanded")
        // Disclosure state survives the collapsed outline.
        driver.jump = TranscriptJump(id: "u0", token: 81); await spin(3); materialize(window); await spin(3)
        if cell(document, "w0") == nil { fail("work expansion lost across outline collapse") }
        note("OUTLINE-EXPAND ok")

        // A scroll during the animation ends it at its final geometry.
        driver.jump = TranscriptJump(id: "u120", token: 82); await spin(3); materialize(window); await spin(3)
        driver.anchor = nil; driver.collapsed = true; await spin(2)
        clip.setBoundsOrigin(NSPoint(x: 0, y: clip.bounds.minY + 40)); scroll.reflectScrolledClipView(clip); materialize(window); await spin(2)
        await settle()
        checkGeometry(document, scroll, "outline-interrupted")
        if cells(document).contains(where: { $0.rowID?.hasPrefix("a") == true }) { fail("interrupted collapse kept assistant rows") }
        driver.anchor = nil; driver.collapsed = false; await settle()
        checkGeometry(document, scroll, "outline-toolbar-expanded")
        note("OUTLINE-INTERRUPT ok")
        note("PASS")
        exit(0)
    }
}
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--swift-build', type=Path, required=True, help='absolute SwiftPM debug build directory')
    parser.add_argument('--library', type=Path, required=True, help='absolute libvelune_bindings.dylib path')
    parser.add_argument('--pairs', type=int, default=400)
    parser.add_argument('--snapshot', type=Path, help='existing directory for PNG snapshots of the viewport')
    parser.add_argument('--keep', type=Path, help='copy the compiled probe here for profiling')
    args = parser.parse_args()
    repo = Path(__file__).resolve().parent.parent
    build = args.swift_build
    with tempfile.TemporaryDirectory(prefix='velune-transcript-scroll-') as temporary:
        root = Path(temporary)
        (root / 'Main.swift').write_text(MAIN)
        objects = [str(p) for name in ('VeluneBindings', 'Markdown', 'CAtomic', 'cmark_gfm', 'cmark_gfm_extensions') for p in (build / (name + '.build')).rglob('*.o')]
        cmd = ['xcrun', 'swiftc', '-parse-as-library', '-swift-version', '5', '-warnings-as-errors', '-O',
               '-I', str(build / 'Modules'), '-I', str(repo / 'target/swift-ffi'), '-Xcc', '-I' + str(repo / '.build/checkouts/swift-cmark/src/include')]
        for path in (repo / '.build/checkouts/swift-markdown/Sources/CAtomic/include/module.modulemap',
                     repo / '.build/checkouts/swift-cmark/src/include/module.modulemap',
                     repo / '.build/checkouts/swift-cmark/extensions/include/module.modulemap'):
            cmd += ['-Xcc', '-fmodule-map-file=' + str(path)]
        sources = ('Models.swift', 'TranscriptModel.swift', 'TranscriptOutline.swift', 'Transcript.swift',
                   'TranscriptTable.swift', 'TranscriptLayout.swift', 'MarkdownRendering.swift')
        cmd += [str(repo / 'app/mac' / name) for name in sources] + [str(root / 'Main.swift'), *objects,
                '-L', str(args.library.parent), '-lvelune_bindings', '-Xlinker', '-rpath', '-Xlinker', str(args.library.parent),
                '-o', str(root / 'Probe')]
        if subprocess.run(cmd, cwd=repo).returncode != 0: raise SystemExit('probe compilation failed')
        if args.keep: shutil.copy2(root / 'Probe', args.keep)
        env = dict(os.environ, PROBE_PAIRS=str(args.pairs))
        if args.snapshot: env['PROBE_SNAPSHOT'] = str(args.snapshot); env['PROBE_VISIBLE'] = '1'
        if subprocess.run([str(root / 'Probe')], cwd=root, env=env, timeout=300).returncode != 0: raise SystemExit('probe failed')


if __name__ == '__main__':
    main()
