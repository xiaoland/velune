#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."

# tiann/tunwg v26.08.03+122a6d0, checked against GitHub's release asset digests.
revision=122a6d0e43585bed8782ceb893394104b6640589
case "$(uname -m)" in
  arm64)
    asset=tunwg-darwin-arm64
    digest=4ff96e3ff7271a1bb940c8e7e071900d2135b483baab387d4f8ba8dc2ddd333a ;;
  x86_64)
    asset=tunwg-darwin
    digest=5b354c7d071cf7c5346f446fbbec8f4fe218278c6d1d9c6dca217dcdd0b96c22 ;;
  *) echo '当前 Mac 架构没有 HAPI 中继组件。' >&2; exit 1 ;;
esac

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
  "https://github.com/tiann/tunwg/releases/download/v26.08.03%2B122a6d0/$asset" -o "$temporary"
printf '%s  %s\n' "$digest" "$temporary" | shasum -a 256 -c --status
chmod 755 "$temporary"
mv -f -- "$temporary" "$destination"
printf '%s\n' "$destination"
