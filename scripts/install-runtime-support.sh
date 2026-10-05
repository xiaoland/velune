#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
node -e 'const [major,minor]=process.versions.node.split(".").map(Number); if (major < 22 || (major === 22 && minor < 18)) { console.error("huihua requires Node 22.18+"); process.exit(1); }'
runtime="$PWD/target/runtime-support"
mkdir -p "$runtime"
cp packages/agent-runtime/runtime-support/package{,-lock}.json "$runtime/"
npm ci --prefix "$runtime" --ignore-scripts --omit=dev --no-audit --no-fund
cp -R packages/agent-runtime/runtime-support/licenses "$runtime/"
cp packages/agent-runtime/runtime-support/THIRD_PARTY_NOTICES.md "$runtime/"
printf 'Installed isolated history reader dependencies in %s\n' "$runtime"
