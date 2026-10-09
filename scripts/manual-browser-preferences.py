#!/usr/bin/env python3
"""Manual isolated AppStore → Transport → UniFFI browser preference persistence; not CI."""
import shutil
import argparse
from pathlib import Path
import subprocess
import tempfile

SWIFT = r'''
import Foundation
import Darwin
import VeluneBindings
@main struct Manual {
    static func require(_ condition: @autoclosure () -> Bool, _ detail: String) throws {
        guard condition() else { throw NSError(domain: "ManualBrowserPreferences", code: 1, userInfo: [NSLocalizedDescriptionKey:detail]) }
    }
    @MainActor static func wait(_ condition: @escaping () -> Bool) async throws {
        let deadline = Date().addingTimeInterval(20)
        while !condition() && Date() < deadline { try await Task.sleep(nanoseconds:20_000_000) }
        try require(condition(), "operation timeout")
    }
    @MainActor static func main() async {
        do { try await run() } catch { print("FAIL: \(error)"); exit(1) }
    }
    @MainActor static func run() async throws {
        let root = URL(fileURLWithPath:CommandLine.arguments[1])
        let home = root.appendingPathComponent("home"), resources = root.appendingPathComponent("resources")
        let transport = Transport(stateDirectory:home, resourcesDirectory:resources)
        let store = AppStore(transport:transport)
        try require(store.conversationBrowserPreferences == nil, "unloaded default would overwrite persistence")
        store.start(); try await wait { !store.isLoading }
        let initial = store.conversationBrowserPreferences!
        try require(initial == ConversationBrowser().preferences, "empty-home defaults")
        try require(store.problems.isEmpty, "empty source caused error")
        for value in [
            BindingConversationBrowserPreferences(grouping:.runtime, sort:.updated, oldestFirst:false, runtimeId:nil, project:.unspecified),
            BindingConversationBrowserPreferences(grouping:.none, sort:.created, oldestFirst:true, runtimeId:nil, project:.all)
        ] {
            store.setConversationBrowserPreferences(value)
            try await wait { (try? transport.list().conversationBrowserPreferences) == value && store.conversationBrowserPreferences == value }
        }
        let final = BindingConversationBrowserPreferences(grouping:.project, sort:.created, oldestFirst:true, runtimeId:"unavailable-synthetic-instance", project:.path(path:"/synthetic/nonexistent/project"))
        store.refreshConversations() // Queued old list must not reset newer optimistic choices.
        store.setConversationBrowserPreferences(.init(grouping:.runtime, sort:.updated, oldestFirst:false, runtimeId:nil, project:.unspecified))
        store.setConversationBrowserPreferences(.init(grouping:.none, sort:.created, oldestFirst:false, runtimeId:nil, project:.all))
        store.setConversationBrowserPreferences(final)
        try require(store.conversationBrowserPreferences == final, "rapid choices not immediately visible")
        try await wait { !store.isLoading }
        try await wait { (try? transport.list().conversationBrowserPreferences) == final }
        try require(store.conversationBrowserPreferences == final, "old completion reverted latest choices")
        var browser = ConversationBrowser(); browser.search = "temporary search"; browser.restore(final)
        try require(browser.preferences == final && browser.search == "temporary search", "restore changed transient search")
        _ = try transport.setConversationBrowserGroupLimit(37)
        var closed = false; store.shutdown { closed = $0 }; try await wait { closed }
        let reopened = Transport(stateDirectory:home, resourcesDirectory:resources)
        let next = AppStore(transport:reopened); next.start(); try await wait { !next.isLoading }
        try require(next.conversationBrowserPreferences == final && next.conversationBrowserGroupLimit == 37, "restart lost persisted choices")
        let configuration = home.appendingPathComponent("generic-config.json")
        let json = try JSONSerialization.jsonObject(with:Data(contentsOf:configuration)) as! [String:Any]
        try require((json["schemaVersion"] as? Int) == 7 && json["conversationBrowserPreferences"] != nil, "configuration reset schema")
        let configurationText = try String(contentsOf:configuration,encoding:.utf8)
        try require(!configurationText.contains("temporary search"), "search leaked to persistence")
        // Atomic write failure must restore the last durable choice and show a problem.
        let backup = home.appendingPathComponent("synthetic-config-saved")
        try FileManager.default.moveItem(at:configuration,to:backup)
        try FileManager.default.createDirectory(at:configuration,withIntermediateDirectories:false)
        next.setConversationBrowserPreferences(initial)
        try await wait { !next.problems.isEmpty }
        try require(next.conversationBrowserPreferences == final, "failed commit did not restore durable preference")
        try FileManager.default.removeItem(at:configuration)
        try FileManager.default.moveItem(at:backup,to:configuration)
        let failedCommitState = try reopened.list().conversationBrowserPreferences
        try require(failedCommitState == final, "failed write changed core state")
        closed = false; next.shutdown { closed = $0 }; try await wait { closed }
        print("PASS initial-default / rapid-latest / stale-list / restart-all-fields / schema7 / transient-search / atomic-failure")
    }
}
'''

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--library',type=Path,required=True)
    parser.add_argument('--swift-build',type=Path,required=True)
    args = parser.parse_args()
    for path in (args.library, args.swift_build):
        if not path.is_absolute() or not path.exists(): parser.error("absolute existing paths required")
    repository = Path(__file__).resolve().parent.parent
    with tempfile.TemporaryDirectory(prefix='velune-browser-preferences-') as directory:
        root = Path(directory)
        for name in ('home','resources','frameworks'): (root/name).mkdir()
        (root/'frameworks/libvelune_bindings.dylib').symlink_to(args.library)
        source = root / 'Manual.swift'; source.write_text(SWIFT)
        build = args.swift_build
        objects = [str(path) for name in ('VeluneBindings', 'MarkdownView', 'Markdown', 'Highlightr', 'RichText', 'Introspection', 'SwiftMath', 'CAtomic', 'cmark_gfm', 'cmark_gfm_extensions') for path in (build / (name + '.build')).rglob('*.o')]
        command = ['xcrun', 'swiftc', '-parse-as-library', '-swift-version', '5', '-warnings-as-errors', '-I', str(build / 'Modules'), '-I', str(repository / 'target/swift-ffi')]
        command += ['-Xcc', '-I' + str(repository/'.build/checkouts/swift-cmark/src/include'), '-Xcc', '-fmodule-map-file=' + str(repository/'.build/checkouts/swift-markdown/Sources/CAtomic/include/module.modulemap')]
        for path in (repository / '.build/checkouts/swift-cmark/src/include/module.modulemap', repository / '.build/checkouts/swift-cmark/extensions/include/module.modulemap'):
            command += ['-Xcc', '-fmodule-map-file=' + str(path)]
        import_models = root / 'ImportModels.swift'
        import_models.write_text((repository / 'app/mac/ProviderImport.swift').read_text().split('struct ProviderImportView: View {')[0])
        command += [str(repository / 'app/mac' / name) for name in ('Models.swift', 'TranscriptModel.swift', 'BindingMapping.swift', 'ConversationBrowser.swift', 'Problems.swift', 'Store.swift', 'Transport.swift')]
        command += [str(import_models)]
        command += [str(source), *objects, '-L', str(root / 'frameworks'), '-lvelune_bindings', '-Xlinker', '-rpath', '-Xlinker', str(root / 'frameworks'), '-o', str(root / 'manual')]
        subprocess.run(command, check=True, cwd=repository)
        for name in ('Highlightr_Highlightr.bundle', 'SwiftMath_SwiftMath.bundle'):
            shutil.copytree(build/name, root/name)
        env = {'HOME':str(root/'home'),'VELUNE_HOME':str(root/'home'),'PATH':'/usr/bin:/bin'}
        subprocess.run([str(root/'manual'),str(root)],env=env,cwd=root,check=True,timeout=60)

if __name__ == '__main__':
    main()
