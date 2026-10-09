#!/usr/bin/env python3
"""Explicit synthetic Mac presentation acceptance, without real app configuration.
Build the Mac SwiftPM product first. This is a temporary manual entry point, not CI.
It measures the derived row/Markdown cache, not on-screen scrolling frame rate.
"""
import shutil
import argparse, json, pathlib, subprocess, tempfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--swift-build', type=pathlib.Path)
args = parser.parse_args()
root = pathlib.Path(__file__).resolve().parent.parent
build = args.swift_build or pathlib.Path(subprocess.check_output(['swift','build','--show-bin-path'],cwd=root,text=True).strip())
source = r'''
import Foundation
import SwiftUI
import AppKit
import MarkdownView
@MainActor func apply(_ model:TranscriptModel, _ messages:[Message]) {
    model.apply(messages,turns:[],items:messages.map { .message(id:$0.id,messageID:$0.id) },outline:messages.filter { $0.role == .user }.map { TranscriptOutlineEntry(id:$0.id,messageID:$0.id) },identities:messages.map { TranscriptMessageIdentity(id:$0.id,messageID:$0.id) })
}
struct CacheProbe: View {
    let document: CachedMarkdownDocument
    var body: some View {
        if let result = document.result { MarkdownView(result) }
        else { MarkdownReader(document.source) { result in
            let _ = document.retain(result)
            MarkdownView(result)
        } }
    }
}
@MainActor func render(_ document: CachedMarkdownDocument) {
    let host = NSHostingController(rootView: CacheProbe(document: document))
    _ = host.sizeThatFits(in: CGSize(width: 700, height: 100000))
    precondition(document.result != nil, "Official MarkdownReader did not supply its result")
}
@main struct ManualPresentation {
    @MainActor static func main() throws {
        let markdown = "# Heading\n\n- item\n- **strong**\n\n> quote\n\n| A | B |\n| --- | --- |\n| one | two |\n\n```swift\nlet value = 42\n```\n"
        var messages = (0..<5000).map { Message(id: "synthetic-\($0)", role: $0.isMultiple(of: 2) ? .user : .assistant, blocks: [.text(markdown)]) }
        let model = TranscriptModel()
        let start = ContinuousClock.now
        apply(model, messages)
        let initial = start.duration(to: .now)
        precondition(model.rows.reduce(0) { $0 + $1.cacheFillCount } == 0, "Offscreen markdown cache was eagerly filled")
        let visible = Array(model.rows.filter { $0.message.role == .assistant }.prefix(20))
        for row in visible { render(row.markdown[0]!) }
        let cachedObjects = visible.map { ObjectIdentifier($0.markdown[0]!) }
        let cachedDocuments = visible.map { $0.markdown[0]!.result!.document }
        let identities = model.rows.map(ObjectIdentifier.init)
        let parses = model.rows.reduce(0) { $0 + $1.cacheFillCount }
        let revision = model.contentRevision
        let repeatStart = ContinuousClock.now
        for _ in 0..<100 { apply(model, messages) }
        let repeated = repeatStart.duration(to: .now)
        precondition(model.contentRevision == revision)
        precondition(model.rows.map(ObjectIdentifier.init) == identities)
        precondition(model.rows.reduce(0) { $0 + $1.cacheFillCount } == parses)
        messages[messages.count-1].blocks[0] = .text(markdown + "\nStreamed tail")
        let tailStart = ContinuousClock.now
        apply(model, messages)
        let tail = tailStart.duration(to: .now)
        precondition(model.rows.map(ObjectIdentifier.init) == identities)
        precondition(visible.map { ObjectIdentifier($0.markdown[0]!) } == cachedObjects)
        for row in visible { render(row.markdown[0]!) }
        precondition(zip(visible,cachedDocuments).allSatisfy { $0.markdown[0]!.result!.document.isIdentical(to:$1) })
        precondition(model.rows.reduce(0) { $0 + $1.cacheFillCount } == parses)
        render(model.rows.last!.markdown[0]!)
        precondition(model.rows.reduce(0) { $0 + $1.cacheFillCount } == parses + 1)
        precondition(model.contentRevision == revision + 1)
        // The upstream parser renders complete and unfinished streaming input.
        precondition(model.rows.last!.markdown[0]!.result!.document.format().contains("Streamed tail"))
        for stream in ["**partial", "```swift\nlet", "| A | B |\n| ---", "- first\n  - nested"] { render(CachedMarkdownDocument(stream, parsed: {})) }
        let grouped = TranscriptModel()
        let sequence = [Message(id:"u",role:.user,blocks:[.text("Synthetic user")]), Message(id:"commentary",role:.assistant,blocks:[.text("Synthetic commentary")]), Message(id:"tool",role:.tool,blocks:[.tool(id:"call",title:"Synthetic tool",state:.completed,output:"Synthetic output")]),Message(id:"final",role:.assistant,blocks:[.text("Synthetic final")])]
        let turn=TranscriptTurn(id:"turn",userMessageID:"u",workMessageIDs:["commentary","tool"],lastMessageID:"final",durationMs:nil,isRunning:false)
        grouped.apply(sequence,turns:[turn],items:[.message(id:"ui:u",messageID:"u"),.work(turnID:"turn"),.message(id:"ui:final",messageID:"final")],outline:[TranscriptOutlineEntry(id:"ui:u",messageID:"u")],identities:sequence.map { TranscriptMessageIdentity(id:"ui:"+$0.id,messageID:$0.id) })
        let rendered=grouped.items(for:grouped.rows)
        precondition(rendered.count == 3)
        if case .work(let work,let members)=rendered[1] { precondition(work.id == "turn" && members.map(\.message.id) == ["commentary","tool"]) } else { preconditionFailure("Authoritative work item lost") }
        let result: [String:Any] = ["rows":5000,"initialMarkdownCacheFills":parses,"initialSeconds":String(describing:initial),"unchangedApplications":100,"unchangedSeconds":String(describing:repeated),"tailChangedCacheFills":1,"tailSeconds":String(describing:tail),"stableRowIdentities":true,"authoritativeWorkPreserved":true,"onScreenScrollFrameRateMeasured":false]
        print(String(decoding:try JSONSerialization.data(withJSONObject:result,options:[.sortedKeys]),as:UTF8.self))
    }
}
'''
with tempfile.TemporaryDirectory(prefix='velune-transcript-manual-') as directory:
    temporary = pathlib.Path(directory)
    main = temporary/'Manual.swift'; main.write_text(source)
    objects = []
    for target in ['MarkdownView','Markdown','Highlightr','RichText','Introspection','SwiftMath','CAtomic','cmark_gfm','cmark_gfm_extensions']:
        objects.extend(str(path) for path in (build/(target+'.build')).rglob('*.o'))
    command = ['xcrun','swiftc','-parse-as-library','-swift-version','5','-warnings-as-errors','-I',str(build/'Modules')]
    command += ['-Xcc', '-I' + str(root/'.build/checkouts/swift-cmark/src/include'), '-Xcc', '-fmodule-map-file=' + str(root/'.build/checkouts/swift-markdown/Sources/CAtomic/include/module.modulemap')]
    for path in [root/'.build/checkouts/swift-cmark/src/include/module.modulemap', root/'.build/checkouts/swift-cmark/extensions/include/module.modulemap']:
        command += ['-Xcc','-fmodule-map-file='+str(path)]
    command += [str(root/'app/mac/Models.swift'),str(root/'app/mac/TranscriptModel.swift'),str(main),*objects,'-o',str(temporary/'manual')]
    subprocess.run(command,check=True,cwd=root)
    for name in ('Highlightr_Highlightr.bundle', 'SwiftMath_SwiftMath.bundle'):
        shutil.copytree(build/name, temporary/name)
    subprocess.run([str(temporary/'manual')],check=True,cwd=temporary)
