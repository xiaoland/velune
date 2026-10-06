#!/usr/bin/env python3
"""Explicit isolated actual AppStore/Transport/UniFFI Problems acceptance, never CI."""
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

SWIFT = r'''
import Foundation
import VeluneBindings
@main struct Manual {
    @MainActor static func wait(_ condition: @escaping () -> Bool) async {
        let deadline = Date().addingTimeInterval(20)
        while !condition() && Date() < deadline { try? await Task.sleep(nanoseconds: 20_000_000) }
        precondition(condition(), "operation did not settle")
    }
    @MainActor static func main() async {
        let args = CommandLine.arguments
        let root = URL(fileURLWithPath: args[1]), resources = URL(fileURLWithPath: args[2])
        let store = AppStore(transport: Transport(stateDirectory: root.appendingPathComponent("home"), resourcesDirectory: resources))
        store.start(); await wait { !store.isLoading }
        precondition(store.problems.isEmpty)
        let invalid = AIProvider(id: "synthetic", name: "Synthetic", protocolID: .chatCompletionsV1, endpoint: "not a url",
            models: [ProviderModel(recordKey: "", providerModelID: "fixture", nickname: "Synthetic")])
        store.saveProvider(invalid, authenticationEdit: .setAPIKey("synthetic-only"))
        await wait { !store.isLoading }
        precondition(store.problems.count == 1)
        let failure = store.problems[0]
        precondition(failure.source == "保存 AI 提供商")
        precondition(failure.kind == "invalid" && failure.code != nil && failure.phase != nil && failure.operationID != nil)
        precondition(failure.diagnosticText.contains(failure.operationID!))
        store.refreshConversations(); await wait { !store.isLoading }
        precondition(store.problems == [failure], "successful read erased retained operation diagnostic")
        store.clearProblem(failure.id); precondition(store.problems.isEmpty)
        let runtime = RuntimeInstance(id: "fixture", name: "Synthetic runtime", typeID: "pi-1.0.2", gatewayID: "default",
            settings: ["binary": args[4], "nodeBinary": args[3], "agentDir": root.appendingPathComponent("runtime").path])
        store.saveRuntimeInstance(runtime)
        await wait { !store.isLoading }
        precondition(store.problems.count == 1 && store.problems[0].activityKey == "history:fixture")
        precondition(store.problems[0].kind == "history" && store.problems[0].operationID == nil)
        let history = store.problems[0]
        store.refreshConversations(); await wait { !store.isLoading }
        precondition(store.problems == [history], "repeated history read duplicated active issue")
        try! FileManager.default.removeItem(at: root.appendingPathComponent("history-fails"))
        store.refreshConversations(); await wait { !store.isLoading }
        precondition(store.problems.isEmpty, "successful native history read did not resolve issue")
        // Explicitly exercise the activity identity rule with typed data after
        // actual Core diagnostics above: per-request IDs must not flood a timer.
        let first = TransportError.diagnostic(kind: "io", detail: "SYNTHETIC_POLL_FAILURE", code: "fixture", phase: "snapshot", operationID: "one")
        let second = TransportError.diagnostic(kind: "io", detail: "SYNTHETIC_POLL_FAILURE", code: "fixture", phase: "snapshot", operationID: "two")
        store.recordProblem(first, source: "读取会话", activityKey: "poll:fixture")
        store.recordProblem(second, source: "读取会话", activityKey: "poll:fixture")
        precondition(store.problems.count == 1 && store.problems[0].operationID == "one")
        store.clearProblems(); precondition(store.problems.isEmpty)
        for index in 0..<105 { store.recordProblem("SYNTHETIC_\(index)") }
        precondition(store.problems.count == 100 && store.problems.first?.detail == "SYNTHETIC_5")
        store.clearProblems()
        var stopped: Bool?
        store.shutdown { stopped = $0 }; await wait { stopped != nil }
        precondition(stopped == true)
        print("{\"acceptance\":\"PASSED\",\"actualTypedProviderFailure\":true,\"successRetainsOperation\":true,\"explicitClear\":true,\"actualNativeHistoryFailure\":true,\"historyDeduplicatesAndResolves\":true,\"typedActivityDeduplicates\":true,\"bounded100\":true}")
    }
}
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('bundle', 'swift-build', 'node', 'pi'):
        parser.add_argument('--' + name, required=True, type=Path)
    args = parser.parse_args()
    for path in vars(args).values():
        if not path.is_absolute() or not path.exists():
            parser.error('absolute existing paths required')
    repository = Path(__file__).resolve().parent.parent
    with tempfile.TemporaryDirectory(prefix='velune-problems-') as directory:
        root = Path(directory)
        for name in ('home', 'runtime', 'resources'):
            (root / name).mkdir()
        resources = root / 'resources'
        original = args.bundle / 'Contents/Resources'
        for path in original.iterdir():
            if path.suffix == '.mjs':
                shutil.copy(path, resources / path.name)
            elif path.name == 'node_modules':
                (resources / path.name).symlink_to(path)
        (root / 'history-fails').touch()
        (resources / 'pi_sessions.mjs').write_text('import {existsSync} from "node:fs";\n' +
            'if(existsSync(' + repr(str(root / 'history-fails')) + ')) process.exit(2);\n' +
            'await import(' + repr((original / 'pi_sessions.mjs').as_uri()) + ');\n')
        main_source = root / 'Manual.swift'
        main_source.write_text(SWIFT)
        import_models = root / 'ImportModels.swift'
        import_models.write_text((repository / 'app/mac/ProviderImport.swift').read_text().split('struct ProviderImportView: View {')[0])
        build = args.swift_build
        objects = [str(path) for name in ('VeluneBindings', 'MarkdownUI', 'NetworkImage', 'cmark_gfm', 'cmark_gfm_extensions')
                   for path in (build / (name + '.build')).rglob('*.o')]
        command = ['xcrun', 'swiftc', '-parse-as-library', '-swift-version', '5', '-warnings-as-errors', '-I', str(build / 'Modules'), '-I', str(repository / 'target/swift-ffi')]
        for path in (repository / '.build/checkouts/swift-cmark/src/include/module.modulemap', repository / '.build/checkouts/swift-cmark/extensions/include/module.modulemap'):
            command += ['-Xcc', '-fmodule-map-file=' + str(path)]
        command += [str(repository / 'app/mac' / name) for name in ('Models.swift', 'TranscriptModel.swift', 'Transport.swift', 'BindingMapping.swift', 'ConversationBrowser.swift', 'Problems.swift', 'Store.swift')]
        command += [str(import_models), str(main_source), *objects, '-L', str(args.bundle / 'Contents/Frameworks'), '-lvelune_bindings', '-Xlinker', '-rpath', '-Xlinker', str(args.bundle / 'Contents/Frameworks'), '-o', str(root / 'manual')]
        subprocess.run(command, check=True, cwd=repository)
        env = {'HOME': str(root / 'home'), 'PATH': str(args.node.parent) + ':/usr/bin:/bin', 'NO_PROXY': '127.0.0.1,localhost'}
        subprocess.run([str(root / 'manual'), str(root), str(resources), str(args.node), str(args.pi)], cwd=root, env=env, check=True, timeout=70)


if __name__ == '__main__':
    main()
