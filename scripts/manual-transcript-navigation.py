#!/usr/bin/env python3
"""Explicit native transcript/link observation using only synthetic messages; never CI."""
import argparse
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import os
import plistlib
import shutil
import signal
import subprocess
import threading

MAIN = r'''
import SwiftUI
import AppKit
import MarkdownUI
import VeluneBindings
@main struct Observation: App {
    var body: some Scene { WindowGroup("Transcript Navigation Fixture") { Fixture() }.defaultSize(width:820,height:600) }
}
struct Fixture: View {
    @StateObject private var model = TranscriptModel()
    @State private var outline = false
    @State private var revision: UInt64 = 0
    @State private var presentation: BindingTranscriptPresentation = .conversation
    @State private var mode = "全部"
    @State private var started = false
    @State private var pasted = ""
    var body: some View {
        TranscriptView(model:model,conversationID:"synthetic",scrollRequest:0,sentAfterUserID:nil,cwd:CommandLine.arguments[1],showsOutline:$outline,activity:nil,presentation:presentation)
            .toolbar {
                Button("会话大纲") { outline = true }
                Menu("人工判别") {
                    Picker("内容", selection:$mode) { ForEach(["全部","段落","代码","表格"],id:\.self) { Text($0) } }
                    Button("追加回复") {
                        var messages = model.rows.map(\.message)
                        messages.append(Message(id:"append-"+String(revision),role:.assistant,blocks:[.text("新增合成回复 \(revision)")]))
                        model.apply(messages); revision += 1
                    }
                    Button("末条增高") {
                        var messages = model.rows.map(\.message)
                        guard let last = messages.last else { return }
                        messages[messages.count-1] = Message(id:last.id,role:last.role,blocks:[.text(last.text+"\n\n"+String(repeating:"增高合成 Markdown **文本**。\n\n",count:15)+"STREAM END")])
                        model.apply(messages)
                    }
                }
                TextField("复制核对",text:$pasted).frame(width:180)
                Picker("模式",selection:$presentation) { Text("对话").tag(BindingTranscriptPresentation.conversation); Text("用户大纲").tag(BindingTranscriptPresentation.userOutline) }
            }
            .onAppear {
                guard !started else { return }; started = true
                model.apply(seed())
            }
            .onChange(of:mode) { _, value in trace("CONTENT "+value); model.apply(seed()) }
    }
    private func trace(_ value: String) { FileHandle.standardError.write(Data(("PHASE "+value+"\n").utf8)) }
    private func seed() -> [Message] {
        var rows: [Message] = []
        for index in 0..<80 {
            let key = String(format:"%03d",index)
            rows.append(Message(id:"u"+key,role:.user,blocks:[.text("目标用户 "+key)]))
            let paragraphs = (0..<(index % 5 == 0 ? 18 : 1)).map { "段落 \($0)：长短混合 Markdown，用于验证实际布局后的远距离定位。 **强调**、`inline code`。" }.joined(separator:"\n\n")
            var text = "# 回复 "+key+"\n\n"+paragraphs
            if mode == "全部" || mode == "代码" { text += "\n\n```swift\nlet sample = \"synthetic\"\nprint(sample)\n```" }
            if mode == "全部" || mode == "表格" { text += "\n\n| 内容 | 状态 |\n| --- | --- |\n| 合成数据 | 已完成 |" }
            if index == 79 { text += "\n\n[网页]("+CommandLine.arguments[2]+") · [本地文件](fixture.html) · [无法打开](velune-manual-unregistered://synthetic-original)" }
            rows.append(Message(id:"a"+key,role:.assistant,blocks:[.text(text)]))
        }
        return rows
    }
}
'''

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--directory',type=Path,required=True)
    parser.add_argument('--swift-build',type=Path,required=True)
    parser.add_argument('--library',type=Path,required=True)
    parser.add_argument('--build-only',action='store_true')
    args = parser.parse_args()
    repository = Path(__file__).resolve().parent.parent
    root = args.directory
    root.mkdir(parents=True,exist_ok=False)
    # Only a directory created by this invocation is disposable.
    process = None
    server = None
    stderr_file = None
    server_running = False
    previous_handler = None
    def interrupted(signum, frame):
        raise KeyboardInterrupt
    try:
        previous_handler = signal.signal(signal.SIGTERM, interrupted)
        (root/'home').mkdir()
        (root/'fixture.html').write_text('<h1>SYNTHETIC LOCAL LINK</h1>')
        app = root/'Transcript Navigation Fixture.app'
        binary = app/'Contents/MacOS/Observation'
        binary.parent.mkdir(parents=True)
        (app/'Contents/Info.plist').write_bytes(plistlib.dumps({'CFBundleExecutable':'Observation','CFBundleIdentifier':'local.velune.transcript-navigation-fixture','CFBundleName':'Transcript Navigation Fixture','CFBundlePackageType':'APPL','LSMinimumSystemVersion':'14.0'}))
        server = ThreadingHTTPServer(('127.0.0.1',0),partial(SimpleHTTPRequestHandler,directory=str(root)))
        threading.Thread(target=server.serve_forever,daemon=True).start()
        server_running = True
        main_file=root/'Main.swift'; main_file.write_text(MAIN)
        views=(repository/'app/mac/Views.swift').read_text()
        message_file=root/'Messages.swift'; message_file.write_text('import SwiftUI\nimport AppKit\nimport MarkdownUI\n'+views[views.index('struct ConversationMessageView:'):views.index('struct SettingsView:')])
        build=args.swift_build
        objects=[str(p) for name in ('VeluneBindings','MarkdownUI','NetworkImage','cmark_gfm','cmark_gfm_extensions') for p in (build/(name+'.build')).rglob('*.o')]
        command=['xcrun','swiftc','-parse-as-library','-swift-version','5','-module-name','TranscriptNavigationProbe','-warnings-as-errors','-I',str(build/'Modules'),'-I',str(repository/'target/swift-ffi')]
        for p in (repository/'.build/checkouts/swift-cmark/src/include/module.modulemap',repository/'.build/checkouts/swift-cmark/extensions/include/module.modulemap'): command+=['-Xcc','-fmodule-map-file='+str(p)]
        command += [str(repository/'app/mac'/name) for name in ('Models.swift','TranscriptModel.swift','TranscriptOutline.swift','MessageLinks.swift','Transcript.swift')]
        command += [str(message_file),str(main_file),*objects,'-L',str(args.library.parent),'-lvelune_bindings','-Xlinker','-rpath','-Xlinker',str(args.library.parent),'-o',str(binary)]
        subprocess.run(command,cwd=repository,check=True)
        if args.build_only:
            print('STRICT SWIFT BUILD PASS',flush=True)
            return
        print('OBSERVE',app,'HTTP',server.server_port,flush=True)
        print('Outline: jump 040 → 001 → 060; resize and repeat; user-outline expand. Links on user079 reply. Quit closes the sole process and removes this fixture automatically.',flush=True)
        env={'HOME':str(root/'home'),'VELUNE_HOME':str(root/'home'),'PATH':'/usr/bin:/bin'}
        stderr_file = (root/'stderr.log').open('w')
        process = subprocess.Popen([str(binary),str(root),f'http://127.0.0.1:{server.server_port}/fixture.html'],cwd=root,env=env,stderr=stderr_file)
        print('SYNTHETIC PID',process.pid,flush=True)
        result = process.wait()
        print('CHILD EXIT',result,flush=True)
        if result != 0: raise subprocess.CalledProcessError(result,[str(binary)])
    finally:
        if process is not None and process.poll() is None:
            process.terminate()
            try: process.wait(timeout=5)
            except subprocess.TimeoutExpired: process.kill(); process.wait()
        if stderr_file is not None:
            stderr_file.close()
            try:
                diagnostic = (root/'stderr.log').read_text(errors='replace')
                counts = {}
                phase = 'SETUP'
                for line in diagnostic.splitlines():
                    if line.startswith('PHASE '):
                        phase = line.removeprefix('PHASE '); counts.setdefault(phase,0)
                    if 'AttributeGraph: cycle detected' in line: counts[phase] = counts.get(phase,0)+1
                print('LAYOUT CYCLE COUNTS',counts,flush=True)
            except OSError as error:
                print("Diagnostic output unavailable:", error, flush=True)
        if server is not None:
            if server_running: server.shutdown()
            server.server_close()
        shutil.rmtree(root)
        if previous_handler is not None: signal.signal(signal.SIGTERM, previous_handler)

if __name__=='__main__': main()
