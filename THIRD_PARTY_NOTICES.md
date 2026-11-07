# Third-party notices

Velune's source and all platform applications are licensed under the GNU Affero General Public License, version 3 only (AGPL-3.0-only). See `LICENSE` in the source tree, `ThirdParty/LICENSE` in desktop installations, or the Velune license entry in the Android/iOS About screens. Third-party components retain their own licenses; this notice does not replace their license texts.

## New API RelayKit in desktop applications

The macOS and Windows full applications link the RelayKit Go module from [QuantumNous/new-api](https://github.com/QuantumNous/new-api) for protocol conversion. The module version and source commit are pinned in `packages/ai-provider/relaykit-bridge/go.mod` (`v0.3.6-0.20261102115434-45746eee4de2`, upstream commit `45746eee4de25ab1eb75d4b7fbb8de2199483ea3`). The release source archive includes the exact RelayKit module subtree and its Go dependency source under `vendor/`; it does not include the New API management console or frontend.

Copyright (c) QuantumNous and contributors.

Frontend design and development by New API contributors.

The desktop application's About or open-source-license screen preserves the attribution above and a visible link to the original project: <https://github.com/QuantumNous/new-api>.

Velune's integration bridge is in `packages/ai-provider/relaykit-bridge`. The New API management console and its frontend are not included. The pinned upstream license and complete upstream NOTICE are in `app/mac/Licenses/RelayKit` in the source tree and `ThirdParty/RelayKit` in desktop installs; the NOTICE includes additional attribution terms that must travel with desktop distributions.

## Other bundled components

- Rust crate license inventory and texts are generated for each application target and feature set as `ThirdParty/rust-licenses.txt`. UniFFI uses MPL-2.0; its full text is included in the inventory.
- macOS Swift packages and Mermaid's bundled JavaScript component have notices and license texts under `ThirdParty/Swift` and `ThirdParty/Mermaid`. The package lock files are retained with desktop application notices. The source archive carries the Mermaid source map plus its exact upstream snapshot and preferred-source supplements for generated JavaScript inputs; the source and version inventory is documented under `app/mac/Licenses/Mermaid`.
- The bundled xURL command-line tool is built from the source revision pinned in `scripts/install-xurl.sh` and `scripts/install-xurl.ps1`; its Apache-2.0 license is shipped with the tool.
- Runtime-support JavaScript notices and license texts are under `ThirdParty/licenses`. Windows OpenTelemetry notices are under `ThirdParty/OpenTelemetry`.
- Windows NuGet package license texts, notices, and `packages.lock.json` are under `ThirdParty/NuGet` in the Windows application bundle. That collection is generated from the package graph restored for the build.
- Android and iOS are remote-only controller applications. Their feature-specific Rust license inventories and platform dependency notices are shipped separately; these apps do not link RelayKit.

A released platform's About/open-source-license screen links to its matching source archive using its `v<SemVer>` release tag. The release pipeline publishes `velune-<tag>-source.tar.gz` for that tag and full Git commit and records the binary platforms it covers in `SOURCE-ARCHIVE.json`. The archive contains locked Rust crate sources, including unmodified MPL-2.0 UniFFI sources, exact Android source JARs, pinned Swift package sources, npm package tarballs, and Mermaid's exact source map, upstream source snapshot, and preferred-source supplements when those platforms are included; it includes the pinned RelayKit Go module and dependencies for macOS. [`SOURCE-RELEASE.md`](SOURCE-RELEASE.md) documents its contents and creation command. Windows NuGet package licenses and notices are generated into `ThirdParty/NuGet`; Windows source coverage remains unresolved, and the source helper refuses Windows in its requested platform set.
