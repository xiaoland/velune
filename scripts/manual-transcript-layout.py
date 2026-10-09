#!/usr/bin/env python3
"""Explicit offscreen native-layout diagnostic; synthetic content only, never CI."""
import argparse
from pathlib import Path
import os
import shutil
import subprocess
import tempfile

MAIN = r'''
import SwiftUI
import AppKit
import VeluneBindings
import MarkdownView
@MainActor func settle(_ window: NSWindow, ready: () -> Bool = { true }) async {
    let deadline=ContinuousClock.now.advanced(by:.seconds(5))
    while true {
        window.contentView?.layoutSubtreeIfNeeded()
        await withCheckedContinuation { continuation in DispatchQueue.main.async { continuation.resume() } }
        let table=window.contentView.flatMap { descendants($0).compactMap { $0 as? NSTableView }.first }
        if let table {
            // An unordered window does not necessarily draw every visible cell.
            // Materialize only the actual viewport through NSTableView's public API.
            let range=table.rows(in:table.visibleRect)
            if range.location != NSNotFound { for i in range.location..<min(range.location+range.length,table.numberOfRows) { _=table.view(atColumn:0,row:i,makeIfNecessary:true) } }
            if !manualTranscriptPending(table), ready() { return }
        }
        if ContinuousClock.now >= deadline { trace("PENDING "+manualTranscriptPendingDescription(table)); preconditionFailure("native content/height transaction did not complete") }
        try? await Task.sleep(for:.milliseconds(1))
    }
}
@MainActor func descendants(_ view:NSView) -> [NSView] { [view]+view.subviews.flatMap(descendants) }
@MainActor func trace(_ value:String) { FileHandle.standardError.write(Data(("PHASE "+value+"\n").utf8)) }
@MainActor func flat(_ model:TranscriptModel,_ messages:[Message]) {
    model.apply(messages,turns:[],items:messages.map { .message(id:$0.id,messageID:$0.id) },outline:messages.filter { $0.role == .user }.map { .init(id:$0.id,messageID:$0.id) },identities:messages.map { .init(id:$0.id,messageID:$0.id) })
}
@MainActor final class Driver:ObservableObject { @Published var request:UInt64=0 }
struct Harness:View {
    @ObservedObject var driver:Driver
    @ObservedObject var model:TranscriptModel
    @State var showsOutline=false
    var body:some View { TranscriptView(model:model,conversationID:"isolated",scrollRequest:driver.request,sentAfterUserID:nil,cwd:nil,showsOutline:$showsOutline,activity:nil,presentation:.conversation) }
}
@main struct Probe {
    @MainActor static func main() async {
        let app=NSApplication.shared; app.setActivationPolicy(.prohibited)
        let model=TranscriptModel();let driver=Driver()
        var messages:[Message]=[]
        let paragraph=String(repeating:"合成段落文字 **强调**、`代码`，稳定宽度排版检查。",count:6)
        for i in 0..<120 {
            messages.append(Message(id:"u\(i)",role:.user,blocks:[.text("合成用户 \(i)")]))
            messages.append(Message(id:"a\(i)",role:.assistant,blocks:[.text("## 合成回复 \(i)\n\n"+String(repeating:paragraph+"\n\n",count:i % 5 == 0 ? 12 : 2)+"```swift\nprint(42)\n```\n\n| 列 | 值 |\n| --- | --- |\n| 合成 | 42 |")]))
        }
        flat(model,messages)
        let window=NSWindow(contentRect:NSRect(x:0,y:0,width:820,height:600),styleMask:[.titled,.resizable],backing:.buffered,defer:false)
        window.contentView=NSHostingView(rootView:Harness(driver:driver,model:model))
        // Deliberately never orderFront: exercises actual AppKit layout without a GUI window.
        await settle(window)
        guard let table=descendants(window.contentView!).compactMap({$0 as? NSTableView}).first,let scroll=table.enclosingScrollView else { fatalError("native table absent") }
        func firstID() -> Int { table.row(at:NSPoint(x:0,y:scroll.contentView.bounds.minY)) }
        func jump(_ id:String) async { manualTranscriptJump(table,id);await settle(window,ready:{ manualTranscriptAtTarget(table,id) }) }
        trace("INITIAL rows=\(table.numberOfRows) height=\(table.bounds.height)")
        // Repeat fixed-content native layout; selection is exercised only if an NSTextView actually exists.
        await jump("a119")
        let original=table.rect(ofRow:239).height
        var selectedTextViews=0
        for _ in 0..<8 {
            if let cell=table.view(atColumn:0,row:239,makeIfNecessary:false) {
                for text in descendants(cell).compactMap({$0 as? NSTextView}) {
                    selectedTextViews += 1
                    let length=(text.string as NSString).length
                    text.setSelectedRange(NSRange(location:max(0,length-3),length:min(length,3)))
                }
            }
            await settle(window)
            precondition(abs(table.rect(ofRow:239).height-original)<1,"selection changed native row height")
        }
        trace("FIXED_CONTENT stableHeight=\(original) selectionTargets=\(selectedTextViews) (zero means physical SwiftUI Text selection unverified)")
        // Continuous native upward movement materializes previously unseen rows.
        manualTranscriptReadIntent(table)
        await settle(window)
        var previous=scroll.contentView.bounds.minY
        var previousRow=firstID()
        var previousOffset=previous-table.rect(ofRow:previousRow).minY
        for _ in 0..<32 {
            manualTranscriptReadIntent(table)
            scroll.contentView.scroll(to:NSPoint(x:0,y:max(0,previous-350))); scroll.reflectScrolledClipView(scroll.contentView)
            await settle(window)
            let position=scroll.contentView.bounds.minY
            let row=firstID();let offset=position-table.rect(ofRow:row).minY
            if !(row<previousRow || (row==previousRow && offset<=previousOffset+1)) { trace("REVERSE previous=\(previousRow)/\(previousOffset) new=\(row)/\(offset)"); preconditionFailure("upward native movement reversed logical reading position") }
            previous=position;previousRow=row;previousOffset=offset
        }
        trace("UPWARD row=\(firstID()) y=\(previous)")
        // Row content changes while reading must not move the existing reading anchor.
        let anchor=firstID(); let offset=scroll.contentView.bounds.minY-table.rect(ofRow:anchor).minY
        messages[messages.count-1]=Message(id:"a119",role:.assistant,blocks:[.text(String(repeating:paragraph+"\n\n",count:80)+"TAIL END")])
        flat(model,messages); await settle(window)
        trace("GROWTH first=\(firstID()) expected=\(anchor) offset=\(scroll.contentView.bounds.minY-table.rect(ofRow:anchor).minY) previousOffset=\(offset)")
        precondition(firstID()==anchor && abs(scroll.contentView.bounds.minY-table.rect(ofRow:anchor).minY-offset)<1,"content growth moved reading anchor")
        // Native width changes must grow and shrink the same long cell.
        await jump("a119")
        trace("BEFORE_RESIZE first=\(firstID()) cell=\(String(describing:table.view(atColumn:0,row:239,makeIfNecessary:false)?.frame))")
        let wide=table.rect(ofRow:239).height
        window.setContentSize(NSSize(width:600,height:600));await settle(window);await jump("a119")
        let narrow=table.rect(ofRow:239).height
        trace("NARROW host="+manualTranscriptGeometry(table,239)+" body=\(manualBodyWidth)")
        if let cell=table.view(atColumn:0,row:239,makeIfNecessary:false) { trace("NARROW textWidths="+descendants(cell).compactMap { $0 as? NSTextView }.map { String(describing:$0.bounds.width) }.prefix(3).joined(separator:",")) }
        window.setContentSize(NSSize(width:1000,height:600));await settle(window);await jump("a119")
        let wider=table.rect(ofRow:239).height
        trace("WIDER host="+manualTranscriptGeometry(table,239)+" body=\(manualBodyWidth)")
        if let cell=table.view(atColumn:0,row:239,makeIfNecessary:false) { trace("WIDER textWidths="+descendants(cell).compactMap { $0 as? NSTextView }.map { String(describing:$0.bounds.width) }.prefix(3).joined(separator:",")) }
        trace("RESIZE wide=\(wide) narrow=\(narrow) wider=\(wider) column=\(table.tableColumns[0].width) nativeRect=\(table.frameOfCell(atColumn:0,row:239).width) cell=\(String(describing:table.view(atColumn:0,row:239,makeIfNecessary:false)?.frame))")
        precondition(narrow>wide && wider<narrow,"native width/height did not respond both directions")
        window.setContentSize(NSSize(width:600,height:600));await settle(window);await jump("a119")
        precondition(abs(table.rect(ofRow:239).height-narrow)<1,"repeat narrow generation changed actual height")
        trace("REPEAT_NARROW height=\(table.rect(ofRow:239).height)")
        driver.request += 1;await settle(window)
        precondition(abs(scroll.contentView.bounds.maxY-table.bounds.maxY)<1,"explicit request did not resume following")
        messages[messages.count-1]=Message(id:"a119",role:.assistant,blocks:[.text(String(repeating:paragraph+"\n\n",count:90)+"FOLLOW TAIL END")])
        flat(model,messages);await settle(window)
        precondition(abs(scroll.contentView.bounds.maxY-table.bounds.maxY)<1,"same-row intrinsic growth did not follow the tail")
        trace("FOLLOW same-row growth at bottom")
        manualTranscriptReadIntent(table);await settle(window)
        // Work members are independent table rows after expansion.
        messages[203]=Message(id:"a101",role:.assistant,blocks:[.tool(id:"synthetic",title:"合成工具",state:.completed,output:String(repeating:"工具输出行\n",count:40))])
        let work=messages.suffix(40).map(\.id)
        let turn=TranscriptTurn(id:"work",userMessageID:messages[198].id,workMessageIDs:work,lastMessageID:nil,durationMs:nil,isRunning:false)
        model.apply(messages,turns:[turn],items:messages.prefix(200).map { .message(id:$0.id,messageID:$0.id) }+[.work(turnID:"work")],outline:[],identities:messages.map { .init(id:$0.id,messageID:$0.id) })
        await settle(window); await jump("work:work")
        guard let cell=table.view(atColumn:0,row:200,makeIfNecessary:false),let button=descendants(cell).compactMap({$0 as? NSButton}).first(where:{$0.bezelStyle == .disclosure}) else { fatalError("work disclosure absent") }
        button.performClick(nil);await settle(window)
        precondition(table.numberOfRows==241,"expanded work not virtualized into native rows")
        await jump("a101")
        guard let toolCell=table.view(atColumn:0,row:204,makeIfNecessary:false),let toolButton=descendants(toolCell).compactMap({$0 as? NSButton}).first(where:{$0.bezelStyle == .disclosure}) else { fatalError("tool disclosure absent") }
        let toolClosed=table.rect(ofRow:204).height
        toolButton.performClick(nil);trace("TOOL clicked state=\(toolButton.state.rawValue)");await settle(window,ready:{ table.rect(ofRow:204).height>toolClosed })
        let toolOpen=table.rect(ofRow:204).height
        precondition(toolOpen>toolClosed,"local tool expansion did not invalidate intrinsic height")
        toolButton.performClick(nil);await settle(window,ready:{ abs(table.rect(ofRow:204).height-toolClosed)<1 })
        precondition(abs(table.rect(ofRow:204).height-toolClosed)<1,"local tool collapse did not restore height")
        trace("TOOL local closed=\(toolClosed) open=\(toolOpen)")
        await jump("work:work")
        guard let header=table.view(atColumn:0,row:200,makeIfNecessary:false),let collapse=descendants(header).compactMap({$0 as? NSButton}).first(where:{$0.bezelStyle == .disclosure}) else { fatalError("work header absent after reuse") }
        collapse.performClick(nil);await settle(window)
        precondition(table.numberOfRows==201,"work collapse failed")
        trace("WORK expanded=241 collapsed=201")
        let idleDeadline=ContinuousClock.now.advanced(by:.seconds(5))
        while true {
            let height=table.bounds.height;let reports=manualHeightReports;let reads=manualHeightReads
            try? await Task.sleep(for:.seconds(1))
            await settle(window)
            trace("IDLE reports=\(reports)→\(manualHeightReports) reads=\(reads)→\(manualHeightReads)")
            precondition(abs(table.bounds.height-height)<1,"idle native height changed")
            if manualHeightReports==reports && manualHeightReads==reads { break }
            precondition(ContinuousClock.now<idleDeadline,"intrinsic notification stream never became idle")
        }
        trace("PASS native offscreen only; physical mouse/wheel remains unverified")
        window.close()
    }
}
'''

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--swift-build',type=Path,required=True)
    parser.add_argument('--library',type=Path,required=True)
    args=parser.parse_args();repo=Path(__file__).resolve().parent.parent
    with tempfile.TemporaryDirectory(prefix='velune-native-layout-') as temporary:
        root=Path(temporary);build=args.swift_build.resolve();library=args.library.resolve()
        main_file=root/'Main.swift';main_file.write_text(MAIN)
        views=(repo/'app/mac/Views.swift').read_text();messages=root/'Messages.swift'
        message_source='import SwiftUI\nimport AppKit\nimport MarkdownView\n@MainActor var manualBodyWidth: CGFloat = 0\n'+views[views.index('struct ConversationMessageView:'):views.index('struct SettingsView:')]
        message_source=message_source.replace('.markdownCodeBlockStyle(MessageCodeBlockStyle())','.markdownCodeBlockStyle(MessageCodeBlockStyle()).background(GeometryReader { p in Color.clear.onAppear { if row.message.id == "a119" { manualBodyWidth=p.size.width } }.onChange(of:p.size) { _, size in if row.message.id == "a119" { manualBodyWidth=size.width } } })')
        messages.write_text(message_source)
        for name in ('Highlightr_Highlightr.bundle','SwiftMath_SwiftMath.bundle'):shutil.copytree(build/name,root/name)
        objects=[str(p) for name in ('VeluneBindings','MarkdownView','Markdown','Highlightr','RichText','Introspection','SwiftMath','CAtomic','cmark_gfm','cmark_gfm_extensions') for p in (build/(name+'.build')).rglob('*.o')]
        cmd=['xcrun','swiftc','-parse-as-library','-swift-version','5','-warnings-as-errors','-I',str(build/'Modules'),'-I',str(repo/'target/swift-ffi'),'-Xcc','-I'+str(repo/'.build/checkouts/swift-cmark/src/include')]
        for path in (repo/'.build/checkouts/swift-markdown/Sources/CAtomic/include/module.modulemap',repo/'.build/checkouts/swift-cmark/src/include/module.modulemap',repo/'.build/checkouts/swift-cmark/extensions/include/module.modulemap'):cmd+=['-Xcc','-fmodule-map-file='+str(path)]
        transcript=root/'Transcript.swift'
        transcript.write_text((repo/'app/mac/Transcript.swift').read_text().replace('import SwiftUI', 'import SwiftUI\nimport Combine').replace('            .background(Color(nsColor: .windowBackgroundColor))', '            .onReceive(manualJumpRequests) { id in following = false; jumpTarget = id }\n            .background(Color(nsColor: .windowBackgroundColor))').replace('let height = ceil(self.host.intrinsicContentSize.height * scale) / scale', 'manualHeightReads += 1; let height = ceil(self.host.intrinsicContentSize.height * scale) / scale').replace('changedHeightIDs.insert(id)', 'manualHeightReports += 1; changedHeightIDs.insert(id)')+"\n@MainActor let manualJumpRequests=PassthroughSubject<String,Never>()\n@MainActor var manualHeightReports=0\n@MainActor var manualHeightReads=0\n@MainActor func manualTranscriptReadIntent(_ table: NSTableView) { guard let c=table.delegate as? NativeTranscriptTable.Coordinator else { return }; let p=c.parent; c.beginReadingInput(); c.update(NativeTranscriptTable(ids:p.ids,contentRevisions:p.contentRevisions,revision:p.revision,layoutKey:p.layoutKey,followsBottom:false,jumpID:nil,bottomRequest:p.bottomRequest,content:p.content,scrollIntent:p.scrollIntent,jumpCompleted:p.jumpCompleted)) }\n"+r"""
extension NativeTranscriptTable.Coordinator {
    func manualPendingDescription() -> String {
        guard let table else { return "table absent" }
        let range=table.rows(in:table.visibleRect)
        let rows=(range.location..<min(range.location+range.length,renderedIDs.count)).map { i -> String in
            let cell=table.view(atColumn:0,row:i,makeIfNecessary:false) as? NativeTranscriptTable.Cell
            return "\(i):intrinsic=\(String(describing:cell?.subviews.first?.intrinsicContentSize)),pending=\(cell?.isHeightPending ?? false),cache=\(currentHeight(renderedIDs[i]) != nil),height=\(String(describing:heights[renderedIDs[i]])),request=\(String(describing:requests[renderedIDs[i]])),cell=\(cell?.representedID ?? "nil"),wanted=\(renderedIDs[i])"
        }.joined(separator:",")
        return "clip=\(String(describing:scroll?.contentView.bounds)) document=\(table.bounds) anchor=\(String(describing:readingAnchor)) jump=\(parent.jumpID ?? "nil") last=\(lastJump ?? "nil") update=\(updateScheduled) layout=\(pendingLayout) height=\(heightCommitScheduled) queue=\(queuedParent != nil) rows="+rows
    }
    func manualPending() -> Bool {
        guard let table else { return true }
        if updateScheduled || pendingLayout || heightCommitScheduled || queuedParent != nil || readingAnchor != nil { return true }
        let range=table.rows(in:table.visibleRect)
        guard range.location != NSNotFound else { return false }
        for i in range.location..<min(range.location+range.length,renderedIDs.count) {
            guard let cell=table.view(atColumn:0,row:i,makeIfNecessary:false) as? NativeTranscriptTable.Cell else { continue }
            if cell.isHeightPending || currentHeight(renderedIDs[i]) == nil { return true }
        }
        return false
    }
}
@MainActor func manualTranscriptPendingDescription(_ table:NSTableView?)->String { (table?.delegate as? NativeTranscriptTable.Coordinator)?.manualPendingDescription() ?? "missing" }
@MainActor func manualTranscriptPending(_ table:NSTableView)->Bool { (table.delegate as? NativeTranscriptTable.Coordinator)?.manualPending() ?? true }
@MainActor func manualTranscriptGeometry(_ table:NSTableView,_ index:Int)->String {
    guard let cell=table.view(atColumn:0,row:index,makeIfNecessary:false) as? NativeTranscriptTable.Cell else { return "absent" }
    let host=cell.subviews.first!
    return "nativeRow=\(table.row(for:cell)) width=\(cell.contentWidth) host=\(host.frame) intrinsic=\(host.intrinsicContentSize) measured=\(cell.measuredHeight)"
}
@MainActor func manualTranscriptJump(_ table:NSTableView,_ id:String) { manualJumpRequests.send(id) }
extension NativeTranscriptTable.Coordinator {
    func manualAtTarget(_ id:String)->Bool {
        guard let table,let scroll,let index=renderedIDs.firstIndex(of:id),currentHeight(id) != nil else { return false }
        let clip=scroll.contentView
        let rect=table.rect(ofRow:index)
        let expected=clip.constrainBoundsRect(NSRect(origin:NSPoint(x:clip.bounds.minX,y:rect.minY+NativeTranscriptTable.contentInset),size:clip.bounds.size))
        return abs(clip.bounds.minY-expected.minY)<1
    }
}
@MainActor func manualTranscriptAtTarget(_ table:NSTableView,_ id:String)->Bool { (table.delegate as? NativeTranscriptTable.Coordinator)?.manualAtTarget(id) ?? false }
""")
        cmd += [str(repo/'app/mac'/name) for name in ('Models.swift','TranscriptModel.swift','TranscriptOutline.swift','MessageLinks.swift')]+[str(transcript)]+[str(messages),str(main_file),*objects,'-L',str(library.parent),'-lvelune_bindings','-Xlinker','-rpath','-Xlinker',str(library.parent),'-o',str(root/'Probe')]
        subprocess.run(cmd,cwd=repo,check=True)
        env={'HOME':str(root),'VELUNE_HOME':str(root),'PATH':'/usr/bin:/bin'}
        subprocess.run([str(root/'Probe')],cwd=root,env=env,check=True,timeout=60)
if __name__=='__main__':main()
