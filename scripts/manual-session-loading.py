#!/usr/bin/env python3
"""Explicit Mac AppStore/Transport loading acceptance with isolated Pi history.
Compiles the production Store plus a same-file visibility shim for one poll;
uses actual UniFFI/core/helper execution, not mocked Store or response models.
No window, credentials, upstream request or automated-test entry point is used.
"""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import tempfile

SWIFT = r'''
import AppKit
import Combine
import Foundation
import VeluneBindings
@main struct ManualLoading {
    @MainActor static func main() throws {
        let args = CommandLine.arguments
        let root = URL(fileURLWithPath: args[1]), resources = URL(fileURLWithPath: args[2])
        let ids = Array(args[3...5]), node = args[6]
        let application = try VeluneApplication.open(options: BindingOptions(homeDirectory: root.appendingPathComponent("application").path, resourcesDirectory: resources.path))
        _ = try application.upsertRuntime(runtime: BindingRuntimeInstance(id: "fixture", name: "Synthetic", typeId: "pi-1.0.2", gatewayId: "default", settings: ["binary":resources.appendingPathComponent("node_modules/@earendil-works/pi-coding-agent/dist/bundle/cli.js").path,"nodeBinary":node,"agentDir":root.appendingPathComponent("runtime").path]))
        _ = try application.selectRuntime(id: "fixture")
        try application.shutdown()
        let store = AppStore(transport: Transport(stateDirectory: root.appendingPathComponent("application"), resourcesDirectory: resources))
        func wait(_ predicate: () -> Bool) {
            let deadline = Date().addingTimeInterval(12)
            while !predicate() && Date() < deadline { RunLoop.main.run(until: Date().addingTimeInterval(0.01)) }
            precondition(predicate(), "operation did not reach expected state")
        }
        store.start()
        wait { store.loadedConversationID == ids[0] && !store.isLoading }
        let firstRows = store.transcript.rows.map(ObjectIdentifier.init)
        var selections: [String?] = []
        let observer = store.$selectedConversationID.sink { selections.append($0) }
        store.manualPoll()
        Thread.sleep(forTimeInterval: 0.15) // Queue the old poll's main-thread delivery.
        selections.removeAll()
        try "hold".write(to: root.appendingPathComponent("gate"), atomically: true, encoding: .utf8)
        store.selectConversation(id: ids[1])
        precondition(store.selectedConversationID == ids[1] && store.pendingConversationID == ids[1])
        wait { FileManager.default.fileExists(atPath: root.appendingPathComponent("ready").path) }
        precondition(store.isLoading && store.loadedConversationID == ids[0])
        precondition(store.transcript.rows.map(ObjectIdentifier.init) == firstRows)
        precondition(selections.allSatisfy { $0 == ids[1] }, "late poll restored old selection")
        store.selectConversation(id: ids[2]) // Serialized: disabled sidebar cannot issue this in the UI.
        precondition(store.selectedConversationID == ids[1], "concurrent selection changed pending target")
        try FileManager.default.removeItem(at: root.appendingPathComponent("gate"))
        wait { !store.isLoading && store.pendingConversationID == nil }
        precondition(store.loadedConversationID == ids[1] && store.selectedConversationID == ids[1])
        precondition(selections.allSatisfy { $0 == ids[1] }, "selection bounced during successful load")
        let loadedRows = store.transcript.rows.map(ObjectIdentifier.init)
        selections.removeAll()
        store.selectConversation(id: ids[2])
        precondition(store.selectedConversationID == ids[2] && store.pendingConversationID == ids[2])
        wait { !store.isLoading && store.pendingConversationID == nil }
        precondition(store.loadedConversationID == ids[1] && store.selectedConversationID == ids[1])
        precondition(store.transcript.rows.map(ObjectIdentifier.init) == loadedRows && store.error != nil)
        precondition(selections == [ids[2],ids[1]], "failed load had unexpected selection transitions")
        observer.cancel()
        var closed = false
        store.shutdown { closed = $0 }; wait { closed }
        print("{\"actualAppStoreAndTransport\":true,\"slowLoadSelectionStable\":true,\"latePollIgnored\":true,\"concurrentSelectionSerialized\":true,\"failedLoadRestoredOnce\":true,\"loadedRowsPreservedDuringLoadAndFailure\":true,\"upstreamRequests\":0}")
    }
}
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('bundle', 'node'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--swift-build', type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    build = args.swift_build or Path(subprocess.check_output(['swift','build','--show-bin-path'],cwd=root,text=True).strip())
    with tempfile.TemporaryDirectory(prefix='velune-session-loading-') as directory:
        temporary = Path(directory)
        resources = temporary / 'resources'; resources.mkdir()
        (resources / 'node_modules').symlink_to(args.bundle / 'Contents/Resources/node_modules')
        (temporary / 'home').mkdir(); (temporary / 'project').mkdir()
        env = {'HOME':str(temporary / 'home'),'PATH':str(args.node.parent) + ':/usr/bin:/bin'}
        seed = resources / 'seed.mjs'
        seed.write_text("""import {SessionManager} from '@earendil-works/pi-coding-agent';
const [cwd,dir,title]=process.argv.slice(2); const manager=SessionManager.create(cwd,dir);
manager.appendMessage({role:'user',content:[{type:'text',text:title}],timestamp:Date.now()});
manager.appendMessage({role:'assistant',content:[{type:'text',text:'SYNTHETIC '+title}],api:'openai-completions',provider:'synthetic',model:'synthetic',stopReason:'stop',timestamp:Date.now()});
manager.appendSessionInfo(title); console.log(manager.getSessionFile());
""")
        sessions = temporary / 'runtime/sessions/project'; sessions.mkdir(parents=True)
        paths = {}
        for name in ['C','B','A']:
            paths[name] = subprocess.check_output([str(args.node),str(seed),str(temporary / 'project'),str(sessions),name],env=env,text=True).strip()
        helper = resources / 'pi_sessions.mjs'
        original_helper = args.bundle / 'Contents/Resources/pi_sessions.mjs'
        helper.write_text('import {existsSync,writeFileSync} from "node:fs";\n'
            + 'const args=process.argv.slice(2), index=args.indexOf("--inspect-session"), path=index < 0 ? null : args[index+1];\n'
            + 'if(path === ' + json.dumps(paths['B']) + ') { writeFileSync(' + json.dumps(str(temporary / 'ready')) + ',"ready"); while(existsSync(' + json.dumps(str(temporary / 'gate')) + ')) await new Promise(resolve=>setTimeout(resolve,15)); }\n'
            + 'if(path === ' + json.dumps(paths['C']) + ') process.exit(1);\n'
            + 'await import(' + json.dumps(original_helper.as_uri()) + ');\n')
        main = temporary / 'Manual.swift'; main.write_text(SWIFT)
        store = temporary / 'Store.swift'
        store.write_text((root / 'app/mac/Store.swift').read_text() + '\nextension AppStore { func manualPoll() { poll() } }\n')
        import_models = temporary / 'ImportModels.swift'
        import_models.write_text((root / 'app/mac/ProviderImport.swift').read_text().split('struct ProviderImportView: View {')[0])
        objects = []
        for name in ['VeluneBindings','MarkdownUI','NetworkImage','cmark_gfm','cmark_gfm_extensions']:
            objects.extend(str(path) for path in (build / (name + '.build')).rglob('*.o'))
        command = ['xcrun','swiftc','-parse-as-library','-swift-version','5','-warnings-as-errors','-I',str(build / 'Modules'),'-I',str(root / 'target/swift-ffi')]
        for path in [root/'.build/checkouts/swift-cmark/src/include/module.modulemap',root/'.build/checkouts/swift-cmark/extensions/include/module.modulemap']:
            command += ['-Xcc','-fmodule-map-file=' + str(path)]
        command += [str(root / 'app/mac' / name) for name in ['Models.swift','TranscriptModel.swift','Transport.swift','BindingMapping.swift']]
        command += [str(import_models),str(store),str(main),*objects,'-L',str(args.bundle / 'Contents/Frameworks'),'-lvelune_bindings','-Xlinker','-rpath','-Xlinker',str(args.bundle / 'Contents/Frameworks'),'-o',str(temporary / 'manual')]
        subprocess.run(command,check=True,cwd=root)
        identities = ['fixture:' + paths[name] for name in ['A','B','C']]
        subprocess.run([str(temporary / 'manual'),str(temporary),str(resources),*identities,str(args.node)],check=True,cwd=temporary,env=env)

if __name__ == '__main__': main()
