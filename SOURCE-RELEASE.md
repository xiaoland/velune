# Corresponding source archives

Each published binary must link to a source archive for the same release tag and full Git commit. The source helper exports that exact commit and adds the locked source dependencies used by the Mac and Android/iOS builds:

- All Rust crates resolved by the workspace `Cargo.lock`, in `vendor/`, plus a generated `.cargo/config.toml` that keeps the repository's existing Cargo settings and redirects crates.io to that vendor tree. The tracked `third-party/iroh` patches remain in the source tree.
- Android `-sources.jar` files for every non-metadata module in `app/android/app/gradle.lockfile`.
- Exact Swift package revision archives from the top-level and iOS `Package.resolved` files.
- npm package tarballs from `packages/agent-runtime/runtime-support/package-lock.json`, checked against each lockfile SRI integrity value.
- The exact Mermaid 11.12.0 npm source map for `app/mac/Assets/Artifacts/mermaid.min.js`; the helper checks that both the npm tarball and app bundle match pinned hashes and includes its embedded `sourcesContent`.
- The exact Mermaid upstream source snapshot, pinned npm preferred sources for the three runtime-support packages shipped as generated JavaScript, 23 source snapshots for generated modules embedded in Mermaid, and the exact protobuf well-known-type `.proto` inputs. `cuint` and `xxhashjs` are already distributed as editable JavaScript in their npm tarballs. The pinned source set and its commit/provenance evidence are listed in `app/mac/Licenses/Mermaid/PREFERRED-SOURCE-SUPPLEMENTS.json`; the 75-module inventory extracted from the actual Mermaid bundle is in `BUNDLED-MODULES.json`.
- The exact RelayKit module subtree and its Go module dependencies vendored from the checked-in `go.mod` and `go.sum`. This is the RelayKit subtree only, not the New API management console or frontend.

Run the helper from a Git checkout with the release tag available locally:

```sh
python3 scripts/create-source-bundle.py \
  --tag "$GITHUB_REF_NAME" \
  --commit "$GITHUB_SHA" \
  --platforms macos windows android \
  --output-dir dist
```

The current release target list includes Windows, so this invocation intentionally fails closed until Windows source coverage is resolved or a Human selects a different release scope and the workflow target list is changed with it.

It writes `dist/velune-<tag>-source.tar.gz`. The helper verifies that the tag resolves to the supplied commit, exports with `git archive`, and writes `SOURCE-ARCHIVE.json` with the corresponding platform set, tag, commit, dependency source counts, tool versions, and SHA-256 digests. It does not read the working tree or include untracked files. `source_tree_sha256` hashes the exported and vendored source tree, excluding only `.git`, `target`, `.build`, `DerivedData`, `dist`, `app/android/.gradle`, `app/android/app/build`, `app/windows/bin`, `app/windows/obj`, and `packages/agent-runtime/runtime-support/node_modules`. Build scripts use this manifest as the source identity when the archive has no `.git` directory.

Archive creation needs the Rust toolchain pinned by `rust-toolchain.toml`, Python 3, Git, and Go 1.25.1 or later. It needs network access to the Cargo registry, Google Maven/Maven Central, GitHub, npm, and the Go module proxy for sources not already cached. Cargo.lock contains registry crate versions and checksums; Android Gradle locking pins module versions; SwiftPM pins repository revisions; npm integrity values pin package tarballs; and `go.sum` verifies Go module content. The helper does not run Gradle or SwiftPM and does not need an Android SDK, Xcode, or a .NET SDK.

The archive contains source and build scripts, not compilers, platform SDKs, operating-system system libraries, or a prebuilt application. A rebuild may still need the relevant platform toolchain and network access for any build-time tools not listed above. Android/iOS controller builds do not include RelayKit. The preferred-source supplements include source inputs, not a promise that every upstream release can be reproduced bit for bit from a clean toolchain.

`app/windows/packages.lock.json` records the exact Windows NuGet restore graph. WindowsAppSDK packages do not publish an exact repository commit in their NuGet metadata, so the helper fails if `windows` is requested and the archive does not claim Windows source coverage. Do not publish a Windows binary until the Windows dependency-source and license path is resolved; see the Windows package notices generated into `ThirdParty/NuGet` by the build.
