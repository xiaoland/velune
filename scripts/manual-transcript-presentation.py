#!/usr/bin/env python3
"""Explicit synthetic Mac presentation acceptance, without real app configuration.
Build the Mac SwiftPM product first. This is a temporary manual entry point, not CI.
It measures the derived row/Markdown cache, not on-screen scrolling frame rate.
"""
import argparse, json, pathlib, subprocess, tempfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--swift-build', type=pathlib.Path)
args = parser.parse_args()
root = pathlib.Path(__file__).resolve().parent.parent
build = args.swift_build or pathlib.Path(subprocess.check_output(['swift','build','--show-bin-path'],cwd=root,text=True).strip())
source = r'''
import Foundation
import MarkdownUI
@main struct ManualPresentation {
    @MainActor static func main() throws {
        let markdown = "# Heading\n\n- item\n- **strong**\n\n> quote\n\n| A | B |\n| --- | --- |\n| one | two |\n\n```swift\nlet value = 42\n```\n"
        var messages = (0..<5000).map { Message(id: "synthetic-\($0)", role: $0.isMultiple(of: 2) ? .user : .assistant, blocks: [.text(markdown)]) }
        let model = TranscriptModel()
        let start = ContinuousClock.now
        model.apply(messages)
        let initial = start.duration(to: .now)
        let identities = model.rows.map(ObjectIdentifier.init)
        let parses = model.rows.reduce(0) { $0 + $1.parseCount }
        let revision = model.contentRevision
        let repeatStart = ContinuousClock.now
        for _ in 0..<100 { model.apply(messages) }
        let repeated = repeatStart.duration(to: .now)
        precondition(model.contentRevision == revision)
        precondition(model.rows.map(ObjectIdentifier.init) == identities)
        precondition(model.rows.reduce(0) { $0 + $1.parseCount } == parses)
        messages[messages.count-1].blocks[0] = .text(markdown + "\nStreamed tail")
        let tailStart = ContinuousClock.now
        model.apply(messages)
        let tail = tailStart.duration(to: .now)
        precondition(model.rows.map(ObjectIdentifier.init) == identities)
        precondition(model.rows.reduce(0) { $0 + $1.parseCount } == parses + 1)
        precondition(model.contentRevision == revision + 1)
        // Cmark handles complete structural Markdown and unfinished streaming input.
        precondition(model.rows.last!.markdown[0]!.renderPlainText().contains("Streamed tail"))
        for stream in ["**partial", "```swift\nlet", "| A | B |\n| ---", "- first\n  - nested"] { _ = MarkdownContent(stream) }
        let result: [String:Any] = ["rows":5000,"initialMarkdownParses":parses,"initialSeconds":String(describing:initial),"unchangedApplications":100,"unchangedSeconds":String(describing:repeated),"tailChangedParses":1,"tailSeconds":String(describing:tail),"stableRowIdentities":true,"onScreenScrollFrameRateMeasured":false]
        print(String(decoding:try JSONSerialization.data(withJSONObject:result,options:[.sortedKeys]),as:UTF8.self))
    }
}
'''
with tempfile.TemporaryDirectory(prefix='velune-transcript-manual-') as directory:
    temporary = pathlib.Path(directory)
    main = temporary/'Manual.swift'; main.write_text(source)
    objects = []
    for target in ['MarkdownUI','NetworkImage','cmark_gfm','cmark_gfm_extensions']:
        objects.extend(str(path) for path in (build/(target+'.build')).rglob('*.o'))
    command = ['xcrun','swiftc','-parse-as-library','-swift-version','5','-warnings-as-errors','-I',str(build/'Modules')]
    for path in [root/'.build/checkouts/swift-cmark/src/include/module.modulemap', root/'.build/checkouts/swift-cmark/extensions/include/module.modulemap']:
        command += ['-Xcc','-fmodule-map-file='+str(path)]
    command += [str(root/'app/mac/Models.swift'),str(root/'app/mac/TranscriptModel.swift'),str(main),*objects,'-o',str(temporary/'manual')]
    subprocess.run(command,check=True,cwd=root)
    subprocess.run([str(temporary/'manual')],check=True,cwd=temporary)
