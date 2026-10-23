#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
revision=24eedb2ff2b141b025efa1a1658fd7f3e88cdd01
destination="$PWD/target/runtime-support/xurl"
cargo install --git https://github.com/Xuanwo/xurl --rev "$revision" --locked --root "$destination" --force xurl-cli
curl --fail --silent --show-error --location "https://raw.githubusercontent.com/Xuanwo/xurl/$revision/LICENSE" -o "$destination/LICENSE"
printf '%s\n' "$revision" > "$destination/REVISION"
"$destination/bin/xurl" --version
