import AppKit
import Foundation
import VeluneBindings

struct AcceptanceError: Error, CustomStringConvertible {
    let description: String
}
@MainActor func require(_ condition: @autoclosure () -> Bool, _ message: String) throws {
    if !condition() { throw AcceptanceError(description: message) }
}
@MainActor func waitFor(_ condition: @escaping () -> Bool) async throws {
    let deadline = Date().addingTimeInterval(20)
    while !condition() && Date() < deadline { try await Task.sleep(for: .milliseconds(50)) }
    try require(condition(), "Timed out waiting for the Store result")
}

// Composer choices are fixture inputs. Core configuration, capability,
// the production observation loop and CLI sampling are not mocked.
@main struct NativeQuotaAcceptance {
    @MainActor static func main() async {
        _ = NSApplication.shared
        NSApp.setActivationPolicy(.prohibited)
        do { try await run() }
        catch { FileHandle.standardError.write(Data("FAIL: \(error)\n".utf8)); exit(1) }
    }
    @MainActor static func run() async throws {
        let root = URL(fileURLWithPath: CommandLine.arguments[1])
        let resources = URL(fileURLWithPath: CommandLine.arguments[2])
        for profile in ["without-provider", "with-provider"] {
            let transport = Transport(stateDirectory: root.appendingPathComponent(profile), resourcesDirectory: resources)
            let store = AppStore(transport: transport)
            store.quotaAcceptanceLoad(try transport.list())
            var observer: Task<Void, Never>?
            do {
                try await waitFor { !store.isLoading && store.runtimeInstances.count == 2 }
                try require(store.runtimeTypes.first { $0.id == "claude-code-2.1" }?.nativeAccountQuota == true, "Core capability did not reach the Store")
                if profile == "without-provider" { try require(store.providers.isEmpty, "Fixture must have no providers") }
                store.quotaAcceptanceSelection(runtimeID: "A", model: .native(providerID: nil, modelID: "sonnet"))
                observer = Task { await store.observeComposerQuota(store.composerQuotaRequest) }
                try await waitFor { store.composerAccountQuotas.first?.quota.freshness == .fresh }
                try require(store.composerAccountQuotas.first?.windows.map(\.usedPercent) == [54, 25], "Native A read another account")
                try require(store.composerAccountQuotas.first?.title.contains("登录额度") == true, "Display must identify login quota")
                observer?.cancel()
                store.quotaAcceptanceSelection(runtimeID: "A", model: nil)
                observer = Task { await store.observeComposerQuota(store.composerQuotaRequest) }
                try await waitFor { store.composerAccountQuotas.first?.quota.freshness == .fresh }
                try require(store.composerAccountQuotas.first?.windows.map(\.usedPercent) == [54, 25], "Runtime default lost its login quota")
                observer?.cancel()
                if profile == "with-provider" {
                    guard let provider = store.providers.first(where: { $0.name == "提供商 B" }), let model = provider.models.first else { throw AcceptanceError(description: "Missing provider fixture") }
                    store.quotaAcceptanceSelection(runtimeID: "A", model: .managed(recordKey: model.recordKey))
                    observer = Task { await store.observeComposerQuota(store.composerQuotaRequest) }
                    try await waitFor { store.composerAccountQuotas.first?.quota.freshness == .fresh }
                    try require(store.composerAccountQuotas.first?.windows.map(\.usedPercent) == [22, 65], "Managed B retained native A")
                    observer?.cancel()
                    store.quotaAcceptanceSelection(runtimeID: "B", model: .native(providerID: nil, modelID: "sonnet"))
                    observer = Task { await store.observeComposerQuota(store.composerQuotaRequest) }
                    try await waitFor { store.composerAccountQuotas.first?.quota.freshness == .fresh }
                    try require(store.composerAccountQuotas.first?.windows.map(\.usedPercent) == [22, 65], "Switching runtime retained A")
                    try require(store.composerAccountQuotas.first?.title == "B · 登录额度", "Display source is ambiguous")
                }
                observer?.cancel()
                var closed = false
                store.shutdown { closed = $0 }
                try await waitFor { closed }
                print("PASS headless Store → production observation → CLI → composer display model: \(profile)")
            } catch {
                observer?.cancel()
                var closed = false
                store.shutdown { closed = $0 }
                try? await waitFor { closed }
                throw error
            }
        }
    }
}
