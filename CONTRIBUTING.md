# Contributing to Velune

Velune is in beta. Bug reports, focused documentation fixes, and discussion of user problems are welcome. For a proposed feature, describe the problem and who it affects before investing in an implementation; an issue or discussion is not a commitment to build it.

## Before opening a pull request

- Discuss larger features or changes to product behavior before investing in an implementation. Small, focused improvements and reproducible bug fixes can go directly to a pull request.
- Send feature pull requests to `develop`. `main` is reserved for promotion, release, and hotfix pull requests.
- Velune's own source is licensed under AGPL-3.0-only; see [`LICENSE`](LICENSE). By submitting a contribution for inclusion, you make your contribution to Velune available under that license. You retain your copyright; this does not relicense third-party material, which keeps its own terms.
- No CLA or DCO sign-off process is required. Submit only work you have the right to license under these terms. Identify third-party material and its license, and do not include material whose terms conflict with the project license.

## Development and verification

Start with `svc --help`, `svc status`, and the [development guide](docs/development.md). Follow the relevant platform README and existing `svc` entry points for setup and builds.

Use synthetic credentials, conversations, and projects. Verify the layers affected by your change and include the evidence that answers the relevant acceptance question. A focused UI change does not require rebuilding and rechecking all four platforms; shared interface or behavior changes may affect more consumers. Record platform limitations instead of implying broader verification.

Do not commit real account state, tokens, API keys, payment data, Harness histories, or session records. Do not post raw diagnostics publicly. Remove secrets and personal data before sharing a diagnostic excerpt; see the [development data boundary](docs/development.md#隔离验证与敏感数据).

## Pull requests

Keep each pull request focused. Explain the user problem, what changed, and how you checked it. Link relevant discussions or issues when available. Update the owning documentation when a durable product behavior or development rule changes. Never include secrets, real session data, or unredacted diagnostics in a patch or screenshot.

## Documentation audiences

Choose the reader before choosing a file:

| Reader and purpose | Owner |
| --- | --- |
| Product users installing, configuring, or using Velune | Root READMEs and [`docs/usage.md`](docs/usage.md) |
| Contributors building, testing, releasing, or debugging | [`docs/development.md`](docs/development.md) and its topic pages |
| Developers understanding a unit or a cross-unit contract | Unit README/TDD and [`docs/product-tdd/`](docs/product-tdd/index.md) |
| Product and implementation work referring to behavior requirements | [`docs/prd/`](docs/prd/index.md) |
| Coding agents following task authority, collaboration, or automation constraints | Root or scoped `AGENTS.md` |

User instructions describe the task and visible result. Developer documentation describes prerequisites, commands, implementation, and diagnostic limits. Coding-agent instructions describe how the agent must operate. A build command can be necessary for a user-facing source-install workflow; its purpose and reader determine placement, not the mere presence of technical terms. Product agents and Harnesses are domain concepts, not coding-agent instructions.

Keep each rule at one owner and link to it elsewhere. PRD owns product behavior; Product TDD owns cross-unit contracts; unit README/TDD owns internal design. Documentation indexes route readers rather than duplicating the rules. Merge superseded statements at their owner and preserve source links; resolve unclear differences before presenting them as current truth. Historical sources and task evidence are not current operating instructions. Beluna is an independent sister project.
