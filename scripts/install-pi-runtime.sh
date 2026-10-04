#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
command -v node >/dev/null
command -v npm >/dev/null
node -e 'const [major,minor]=process.versions.node.split(".").map(Number); if (major < 22 || (major === 22 && minor < 19)) { console.error("Pi requires Node 22.19+"); process.exit(1); }'
runtime="$PWD/target/pi-runtime"
mkdir -p "$runtime"
npm install --prefix "$runtime" --ignore-scripts --no-save --package-lock=true \
  @earendil-works/pi-coding-agent@1.0.2
printf 'Installed isolated Pi SDK/CLI runtime in %s\n' "$runtime"
