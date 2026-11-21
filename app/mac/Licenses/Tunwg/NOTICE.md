# HAPI relay helper (tunwg)

Velune Mac bundles the upstream Darwin release of tiann/tunwg `v26.08.03+122a6d0` (MIT). [Source revision](https://github.com/tiann/tunwg/tree/122a6d0e43585bed8782ceb893394104b6640589) · [Release](https://github.com/tiann/tunwg/releases/tag/v26.08.03%2B122a6d0).

`UPSTREAM.json` records both upstream executable digests and the statically linked Go modules reported in their build information. The corresponding source archive includes the pinned Tunwg source tree and its vendored Go dependencies under `tunwg-sources/`. The dependency notices accompany this file.

The upstream [release workflow](https://github.com/tiann/tunwg/blob/122a6d0e43585bed8782ceb893394104b6640589/.github/workflows/release.yml) creates executables in the checkout's `bin/` directory before building the Darwin variants. The resulting build information reports that checkout as modified. The architecture-specific digests in `UPSTREAM.json` identify the upstream assets before Velune signs them.
