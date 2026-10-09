#!/usr/bin/env python3
"""Explicit synthetic actual-AppStore publication diagnostic; no GUI/CI/real state."""
import argparse
from pathlib import Path
import shutil
import subprocess
import tempfile

MAIN=r'''
import Foundation
import Combine
import VeluneBindings
@main struct Probe {
    @MainActor static func main() async {
        let application=AppStore(preview:true)
        let workspace=application.makeWorkspace()
        var appChanges=0,workspaceChanges=0,transcriptChanges=0
        let app=application.objectWillChange.sink { appChanges+=1 }
        let work=workspace.objectWillChange.sink { workspaceChanges+=1 }
        let transcript=workspace.transcript.objectWillChange.sink { transcriptChanges+=1 }
        for _ in 0..<100 { application.manualIdleCallback();workspace.manualCopy(application) }
        await Task.yield();await Task.yield()
        precondition(appChanges==0 && workspaceChanges==0 && transcriptChanges==0,"unchanged successful callback republished")
        print("IDLE callbacks=100 application=0 workspace=0 transcript=0")
        application.recordProblem("SYNTHETIC_OPERATION",source:"合成操作")
        application.recordProblem(TransportError.rejected("SYNTHETIC_ACTIVE"),source:"合成读取",activityKey:"poll:fixture")
        application.manualIdleCallback()
        for _ in 0..<4 { await Task.yield() }
        precondition(application.problems.count==1 && workspace.problems==application.problems,"active recovery/retained operation not propagated")
        let oldCount=workspaceChanges
        application.saveProvider(AIProvider(id:"synthetic-new",name:"Synthetic",protocolID:.chatCompletionsV1,endpoint:"https://example.invalid/v1",models:[]),authenticationEdit:.keep)
        for _ in 0..<4 { await Task.yield() }
        precondition(workspace.gateway==application.gateway && workspaceChanges>oldCount,"real application config update skipped")
        print("PASS recovery retainedOperation configUpdate propagated; synthetic preview state, exact actual AppStore callback boundary")
        withExtendedLifetime([app,work,transcript]) {}
    }
}
'''

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--swift-build',required=True,type=Path);p.add_argument('--library',required=True,type=Path)
    a=p.parse_args();repo=Path(__file__).resolve().parent.parent;build=a.swift_build.resolve();library=a.library.resolve()
    with tempfile.TemporaryDirectory(prefix='velune-store-idle-') as directory:
        root=Path(directory)
        store=root/'Store.swift';store.write_text((repo/'app/mac/Store.swift').read_text()+'''\nextension AppStore {
            func manualIdleCallback() { clearActivityProblem("poll:fixture"); if let value=snapshot { apply(value,preserveSelection:true) } }
            func manualCopy(_ application:AppStore) { copyApplicationState(application) }
        }\n''')
        imports=root/'ImportModels.swift';imports.write_text((repo/'app/mac/ProviderImport.swift').read_text().split('struct ProviderImportView: View {')[0])
        source=root/'Main.swift';source.write_text(MAIN)
        for name in ('Highlightr_Highlightr.bundle','SwiftMath_SwiftMath.bundle'):shutil.copytree(build/name,root/name)
        objects=[str(f) for name in ('VeluneBindings','MarkdownView','Markdown','Highlightr','RichText','Introspection','SwiftMath','CAtomic','cmark_gfm','cmark_gfm_extensions') for f in (build/(name+'.build')).rglob('*.o')]
        cmd=['xcrun','swiftc','-parse-as-library','-swift-version','5','-warnings-as-errors','-I',str(build/'Modules'),'-I',str(repo/'target/swift-ffi'),'-Xcc','-I'+str(repo/'.build/checkouts/swift-cmark/src/include')]
        for path in (repo/'.build/checkouts/swift-markdown/Sources/CAtomic/include/module.modulemap',repo/'.build/checkouts/swift-cmark/src/include/module.modulemap',repo/'.build/checkouts/swift-cmark/extensions/include/module.modulemap'):cmd+=['-Xcc','-fmodule-map-file='+str(path)]
        cmd += [str(repo/'app/mac'/name) for name in ('Models.swift','TranscriptModel.swift','BindingMapping.swift','ConversationBrowser.swift','Problems.swift','Transport.swift')]+[str(store),str(imports),str(source),*objects,'-L',str(library.parent),'-lvelune_bindings','-Xlinker','-rpath','-Xlinker',str(library.parent),'-o',str(root/'Probe')]
        subprocess.run(cmd,cwd=repo,check=True)
        subprocess.run([str(root/'Probe')],cwd=root,env={'HOME':str(root),'VELUNE_HOME':str(root),'PATH':'/usr/bin:/bin'},check=True,timeout=30)
if __name__=='__main__':main()
