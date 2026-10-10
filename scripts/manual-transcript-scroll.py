#!/usr/bin/env python3
"""Explicit native transcript scroll/geometry diagnostic; synthetic content only, never CI.

Compiles the actual Mac transcript sources into a temporary probe, loads a long
synthetic history into an unordered window and checks: no visible displacement
of the reading row while scrolling fast through unmeasured history, displayed
text geometry equals the measured layout, far jumps, bottom following, width
changes and local disclosure toggles. Prints timings; exits non-zero on failure.
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
@MainActor final class Driver: ObservableObject { @Published var request: UInt64 = 0; @Published var jump: TranscriptJump?; var following = true }
/// Mirrors TranscriptView's inputs, adding explicit jump requests.
struct Harness: View {
    @ObservedObject var model: TranscriptModel
    @ObservedObject var driver: Driver
    var body: some View {
        TranscriptTable(conversationID: "probe", items: model.items(for: model.rows), revision: model.contentRevision, activity: nil,
                        outlineToggleIDs: [], outlineExpandedID: nil, cwd: "/tmp", bottomRequest: driver.request, jump: driver.jump,
                        followingChanged: { driver.following = $0 }, outlineToggle: { _ in })
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
        messages.append(Message(id: "a\(i)", role: .assistant, blocks: [.reasoning("推理 \(i)"), .text(body), .tool(id: "t\(i)", title: "读取文件 \(i)", state: .completed, output: "输出\n第二行")]))
    }
    return messages
}
@MainActor func apply(_ model: TranscriptModel, _ messages: [Message]) {
    model.apply(messages, turns: [], items: messages.map { .message(id: $0.id, messageID: $0.id) },
                outline: messages.filter { $0.role == .user }.map { .init(id: $0.id, messageID: $0.id) },
                identities: messages.map { .init(id: $0.id, messageID: $0.id) })
}
@MainActor func rowTop(_ table: NSTableView, _ scroll: NSScrollView, _ row: Int) -> CGFloat { table.rect(ofRow: row).minY - scroll.contentView.bounds.minY }
@MainActor func materialize(_ window: NSWindow, _ table: NSTableView) {
    window.contentView?.layoutSubtreeIfNeeded()
    let range = table.rows(in: table.visibleRect)
    if range.location != NSNotFound { for i in range.location ..< range.location + range.length { _ = table.view(atColumn: 0, row: i, makeIfNecessary: true) } }
    table.layoutSubtreeIfNeeded()
}
@MainActor func snapshot(_ scroll: NSScrollView, _ name: String) {
    guard let directory = ProcessInfo.processInfo.environment["PROBE_SNAPSHOT"] else { return }
    let view = scroll
    guard let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { return }
    view.cacheDisplay(in: view.bounds, to: rep)
    try? rep.representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: directory).appendingPathComponent(name + ".png"))
}
@MainActor func checkGeometry(_ table: NSTableView, _ label: String) async {
    if let clip = table.enclosingScrollView?.contentView, abs(table.bounds.width - clip.bounds.width) > 0.5 || table.tableColumns[0].width > clip.bounds.width {
        fail("\(label): table \(table.bounds.width) / column \(table.tableColumns[0].width) != viewport \(clip.bounds.width)")
    }
    let range = table.rows(in: table.visibleRect)
    guard range.location != NSNotFound else { return }
    var checked = 0
    for i in range.location ..< range.location + range.length {
        guard let cell = table.view(atColumn: 0, row: i, makeIfNecessary: false) else { continue }
        if abs(cell.frame.height - table.rect(ofRow: i).height) > 0.5 { fail("\(label): cell \(i) frame \(cell.frame.height) != row \(table.rect(ofRow: i).height)") }
        for case let text as TranscriptTextView in descendants(cell) where !text.isHiddenOrHasHiddenAncestor {
            guard let lm = text.layoutManager, let tc = text.textContainer else { continue }
            lm.ensureLayout(for: tc)
            // Displayed TextKit layout must equal the measured frame (ceil of used height).
            let used = lm.usedRect(for: tc).height
            if used > text.frame.height + 0.01 || text.frame.height - used >= 1 { fail("\(label): row \(i) text used \(used) != frame \(text.frame.height)") }
            if text.frame.maxY > cell.bounds.height + 0.5 { fail("\(label): row \(i) text overflows cell") }
            checked += 1
        }
    }
    note("GEOMETRY \(label) rows=\(range.length) texts=\(checked) ok")
}
@main struct Probe {
    @MainActor static func main() async {
        let app = NSApplication.shared; app.setActivationPolicy(.prohibited)
        let model = TranscriptModel(); let driver = Driver()
        let count = Int(ProcessInfo.processInfo.environment["PROBE_PAIRS"] ?? "400")!
        var messages = synthetic(count)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 900, height: 700), styleMask: [.titled, .resizable], backing: .buffered, defer: false)
        window.contentView = NSHostingView(rootView: Harness(model: model, driver: driver))
        await spin(3)
        let clock = ContinuousClock()
        let loadStart = clock.now
        apply(model, messages)
        await spin(1)
        await spin(1)
        guard let table = descendants(window.contentView!).compactMap({ $0 as? NSTableView }).first, let scroll = table.enclosingScrollView else { fail("table absent") }
        materialize(window, table)
        note("LOAD rows=\(table.numberOfRows) firstPaint=\(clock.now - loadStart)")
        let clip = scroll.contentView
        let bottomGap = table.bounds.maxY - clip.bounds.maxY
        if bottomGap > 1 { fail("not at bottom after load gap=\(bottomGap)") }
        await checkGeometry(table, "bottom")

        // Jump into unmeasured history immediately after load.
        driver.jump = TranscriptJump(id: "u7", token: 100); await spin(2); materialize(window, table)
        for _ in 0..<40 { await spin(); try? await Task.sleep(for: .milliseconds(2)) }
        if abs(rowTop(table, scroll, 14) + TranscriptRowLayout.rowInset - 8) > 1 { fail("early jump drifted to \(rowTop(table, scroll, 14))") }
        await checkGeometry(table, "early-jump")
        snapshot(scroll, "early-jump")
        driver.request &+= 1; await spin(3); materialize(window, table)
        if table.bounds.maxY - clip.bounds.maxY > 1 { fail("bottom request after jump") }
        note("EARLY-JUMP ok")

        // Fast upward scroll immediately, while distant rows are still estimates.
        var maxJitter: CGFloat = 0; var worstStep = Duration.zero; var steps = 0
        let stepSize: CGFloat = 260
        while clip.bounds.minY > 0 && steps < 4000 {
            let top = table.row(at: NSPoint(x: 1, y: clip.bounds.minY + clip.bounds.height / 2))
            let before = rowTop(table, scroll, top)
            let start = clock.now
            let delta = min(stepSize, clip.bounds.minY)
            clip.setBoundsOrigin(NSPoint(x: 0, y: clip.bounds.minY - delta))
            scroll.reflectScrolledClipView(clip)
            let expectedDelta = before + delta
            let scrolled = clock.now - start
            materialize(window, table)
            let materialized = clock.now - start
            await spin()
            let elapsed = clock.now - start
            worstStep = max(worstStep, elapsed)
            if elapsed > .milliseconds(40) { note("SLOW step=\(steps) \(elapsed) scroll=\(scrolled) materialize=\(materialized) row=\(top)") }
            let after = rowTop(table, scroll, top)
            let jitter = abs(after - expectedDelta)
            if jitter > 1 && maxJitter <= 1 { note("JITTER step=\(steps) row=\(top) before=\(before) after=\(after) expected=\(expectedDelta) origin=\(clip.bounds.minY)") }
            maxJitter = max(maxJitter, jitter)
            steps += 1
        }
        note("FASTSCROLL steps=\(steps) maxJitter=\(maxJitter) worstStep=\(worstStep)")
        if maxJitter > 1 { fail("reading row moved by \(maxJitter)pt beyond the scroll delta") }
        await checkGeometry(table, "top")

        // Let background measurement finish, then check stability at a mid anchor.
        let settleStart = clock.now
        var lastHeight = table.bounds.height; var stable = 0
        while stable < 30 { await spin(); try? await Task.sleep(for: .milliseconds(5)); if table.bounds.height == lastHeight { stable += 1 } else { stable = 0; lastHeight = table.bounds.height } }
        note("SETTLED documentHeight=\(table.bounds.height) after=\(clock.now - settleStart)")
        clip.setBoundsOrigin(NSPoint(x: 0, y: table.bounds.height / 2)); scroll.reflectScrolledClipView(clip)
        materialize(window, table); await spin(3)
        let midRow = table.row(at: NSPoint(x: 1, y: clip.bounds.minY + 10)); let midTop = rowTop(table, scroll, midRow)
        await spin(10)
        if abs(rowTop(table, scroll, midRow) - midTop) > 0.5 { fail("settled history still moves") }
        await checkGeometry(table, "middle")

        // Width change keeps the reading row.
        window.setContentSize(NSSize(width: 640, height: 700)); materialize(window, table); await spin(3)
        let afterNarrow = rowTop(table, scroll, midRow)
        if abs(afterNarrow - midTop) > 1 { fail("narrowing moved reading row \(midTop) -> \(afterNarrow)") }
        await checkGeometry(table, "narrow")
        window.setContentSize(NSSize(width: 1200, height: 700)); materialize(window, table); await spin(3)
        if abs(rowTop(table, scroll, midRow) - midTop) > 1 { fail("widening moved reading row") }
        await checkGeometry(table, "wide")
        note("RESIZE ok")

        // Tail growth while reading history does not move the reading row.
        messages[messages.count - 1].blocks.append(.text(String(repeating: "尾部增长。", count: 400)))
        apply(model, messages); await spin(3); materialize(window, table)
        if abs(rowTop(table, scroll, midRow) - midTop) > 1 { fail("tail growth moved reading row") }
        note("TAIL-WHILE-READING ok")

        // Local disclosure: the toggled row keeps its place.
        let toolRow = (midRow ..< table.numberOfRows).first { row in
            guard let cell = table.view(atColumn: 0, row: row, makeIfNecessary: false) else { return false }
            return descendants(cell).contains { ($0 as? NSButton)?.bezelStyle == .disclosure && !$0.isHidden && $0.superview?.isHidden == false }
        }
        if let toolRow, let cell = table.view(atColumn: 0, row: toolRow, makeIfNecessary: false),
           let button = descendants(cell).compactMap({ $0 as? NSButton }).last(where: { $0.bezelStyle == .disclosure && $0.superview?.isHidden == false }) {
            let height = table.rect(ofRow: toolRow).height; let top = rowTop(table, scroll, toolRow)
            button.performClick(nil); await spin(3); materialize(window, table)
            if table.rect(ofRow: toolRow).height <= height { fail("disclosure did not grow row") }
            if abs(rowTop(table, scroll, toolRow) - top) > 1 { fail("disclosure moved its row") }
            await checkGeometry(table, "expanded")
            snapshot(scroll, "expanded")
            note("DISCLOSURE ok \(height) -> \(table.rect(ofRow: toolRow).height)")
        } else { fail("no visible disclosure") }

        // Far jumps land the target's top just below the viewport top, also after
        // tail growth and while history is only partly measured.
        for (token, id) in ["u20", "a150", "u390", "u3"].enumerated() {
            driver.jump = TranscriptJump(id: id, token: UInt64(token + 1)); await spin(3); materialize(window, table); await spin(3)
            guard let row = (0 ..< table.numberOfRows).first(where: { messages[$0].id == id }) else { fail("jump target missing") }
            let top = rowTop(table, scroll, row)
            if abs(top + TranscriptRowLayout.rowInset - 8) > 1 { fail("jump \(id) top=\(top)") }
            if driver.following { fail("jump kept following") }
        }
        note("JUMP ok")

        // Whole-message copy is offered from a message's native text menu.
        driver.jump = TranscriptJump(id: "a150", token: 50); await spin(3); materialize(window, table); await spin(3)
        if let row = (0 ..< table.numberOfRows).first(where: { messages.indices.contains($0) && messages[$0].id == "a150" }),
           let cell = table.view(atColumn: 0, row: row, makeIfNecessary: false),
           let text = descendants(cell).compactMap({ $0 as? TranscriptTextView }).first(where: { !$0.isHiddenOrHasHiddenAncestor }),
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
        driver.request &+= 1; await spin(4); materialize(window, table)
        for i in 0..<5 {
            messages[messages.count - 1].blocks.append(.text("流式片段 \(i) " + String(repeating: "内容", count: 80)))
            apply(model, messages); await spin(3); materialize(window, table)
            let gap = table.bounds.maxY - clip.bounds.maxY
            if gap > 1 { fail("following lost bottom gap=\(gap)") }
        }
        await checkGeometry(table, "stream")
        note("FOLLOW ok")
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
        if args.snapshot: env['PROBE_SNAPSHOT'] = str(args.snapshot)
        if subprocess.run([str(root / 'Probe')], cwd=root, env=env, timeout=300).returncode != 0: raise SystemExit('probe failed')


if __name__ == '__main__':
    main()
