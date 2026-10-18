#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."

skip_native=false
case "${1:-}" in
  '') ;;
  --skip-native) skip_native=true; shift ;;
  *) echo '用法：scripts/build-android.sh [--skip-native]' >&2; exit 1 ;;
esac
[[ $# -eq 0 ]] || { echo '用法：scripts/build-android.sh [--skip-native]' >&2; exit 1; }

if [[ -z "${JAVA_HOME:-}" && "$(uname -s)" == Darwin ]]; then
  export JAVA_HOME
  JAVA_HOME=$(/usr/libexec/java_home -v 17)
fi
if [[ -z "${ANDROID_HOME:-}" ]]; then
  export ANDROID_HOME="${ANDROID_SDK_ROOT:-}"
  if [[ -z "$ANDROID_HOME" ]]; then
    case "$(uname -s)" in
      Darwin) export ANDROID_HOME="$HOME/Library/Android/sdk" ;;
      Linux) export ANDROID_HOME="$HOME/Android/Sdk" ;;
    esac
  fi
fi
[[ -d "${ANDROID_HOME:-}" ]] || { echo '请设置 ANDROID_HOME 指向 Android SDK。' >&2; exit 1; }
if [[ "$skip_native" == false ]]; then
  bash scripts/build-android-native.sh --diagnostics
fi
for artifact in target/android-bindings/app/velune/bindings/velune_bindings.kt target/android-native/arm64-v8a/libvelune_bindings.so target/android-native/rustls-platform-verifier-0.2.0.aar; do
  [[ -f "$artifact" ]] || { echo "缺少构建产物：$artifact；请先运行 scripts/build-android-native.sh。" >&2; exit 1; }
done
app/android/gradlew -p app/android :app:assembleDebug :app:lintDebug --console=plain
printf '已构建：%s/app/android/app/build/outputs/apk/debug/app-debug.apk\n' "$PWD"
