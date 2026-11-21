#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."

# The source archive uses the same pin as the packaged binaries.
IFS=$'\t' read -r revision asset digest url < <(python3 - "$(uname -m)" <<'PY'
import json
from pathlib import Path
import sys
from urllib.parse import quote

pin = json.loads(Path("app/mac/Licenses/Tunwg/UPSTREAM.json").read_text(encoding="utf-8"))
asset = pin["assets"].get(sys.argv[1])
if asset is None:
    raise SystemExit("当前 Mac 架构没有 HAPI 中继组件。")
url = f"{pin['repository']}/releases/download/{quote(pin['release'], safe='')}/{asset['name']}"
print(pin["revision"], asset["name"], asset["sha256"], url, sep="\t")
PY
)

destination="$PWD/target/runtime-support/hapi-relay/$revision/$asset"
if [[ -x "$destination" && "$(shasum -a 256 "$destination" | awk '{print $1}')" == "$digest" ]]; then
  printf '%s\n' "$destination"
  exit 0
fi
mkdir -p "$(dirname "$destination")"
temporary=$(mktemp "$destination.XXXXXXXX")
trap 'rm -f -- "$temporary"' EXIT
curl -q --proto '=https' --tlsv1.2 --fail --silent --show-error --location \
  --connect-timeout 10 --max-time 60 \
  "$url" -o "$temporary"
printf '%s  %s\n' "$digest" "$temporary" | shasum -a 256 -c --status
chmod 755 "$temporary"
mv -f -- "$temporary" "$destination"
printf '%s\n' "$destination"
