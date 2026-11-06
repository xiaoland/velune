#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."

destination=/Applications/Velune.app
build=true
while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-build) build=false; shift ;;
    --destination)
      [[ $# -ge 2 && "$2" == /* ]] || { echo '--destination 需要绝对应用路径。' >&2; exit 2; }
      destination="$2"; shift 2 ;;
    *) echo '用法：scripts/reinstall-macos.sh [--skip-build] [--destination /absolute/Velune.app]' >&2; exit 2 ;;
  esac
done
[[ "$(uname -s)" == Darwin ]] || { echo '重装需要 macOS 与 Xcode 工具链。' >&2; exit 1; }

# Keep the current app available throughout the build. A failed build or
# signature check must not interrupt the conversation it is hosting.
if "$build"; then
  echo '重装阶段：构建新版'
  bash scripts/build-macos.sh --profile release --build-only
fi
app="$PWD/target/macos/Velune.app"
echo '重装阶段：验证新版签名'
codesign --verify --deep --strict "$app"
identifier=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$app/Contents/Info.plist")
echo '重装阶段：请求旧版正常退出'
xcrun swift scripts/quit-macos.swift "$identifier" "$destination"
echo '重装阶段：安装新版'
python3 scripts/install-macos.py "$app" --destination "$destination"
home="${VELUNE_HOME:-$HOME/.velune}"
if [[ -d $home ]]; then
  echo '重装阶段：执行数据迁移'
  python3 scripts/run-migrations.py --home "$home"
fi
echo '重装阶段：后台启动新版'
open -g "$destination"
printf '已在后台启动：%s\n' "$destination"
