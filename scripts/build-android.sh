#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
profile=dev
skip_native=false
diagnostics=false
while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-native) skip_native=true; shift ;;
    --diagnostics) diagnostics=true; shift ;;
    --profile) [[ $# -ge 2 ]] || { echo '--profile 需要 dev 或 release。' >&2; exit 1; }; profile="$2"; shift 2 ;;
    *) echo '用法：scripts/build-android.sh [--profile dev|release] [--diagnostics] [--skip-native]' >&2; exit 1 ;;
  esac
done
if [[ "$skip_native" == true && "$diagnostics" == true ]]; then
  echo '--skip-native 沿用已有 native 特性；请去掉 --diagnostics，或执行正常构建以选择诊断配置。' >&2
  exit 1
fi
case "$profile" in
  dev) output_profile=debug; variant=Debug; apk=debug/app-debug.apk ;;
  release) output_profile=release; variant=Release; apk=release/app-release-unsigned.apk ;;
  *) echo '--profile 需要 dev 或 release。' >&2; exit 1 ;;
esac
if [[ "$profile" == release ]]; then
  signing_material=true
  for name in ANDROID_RELEASE_KEYSTORE_PATH ANDROID_RELEASE_STORE_PASSWORD ANDROID_RELEASE_KEY_ALIAS ANDROID_RELEASE_KEY_PASSWORD; do
    [[ -n "${!name:-}" ]] || signing_material=false
  done
  if [[ "${VELUNE_REQUIRE_ANDROID_RELEASE_SIGNING:-}" == true || "$signing_material" == true ]]; then
    apk=release/app-release.apk
  fi
fi
if [[ "${VELUNE_BUILD_MANAGED:-}" != android ]]; then
  arguments=(--profile "$profile")
  [[ "$skip_native" == false ]] || arguments+=(--skip-native)
  [[ "$diagnostics" == false ]] || arguments+=(--diagnostics)
  exec python3 scripts/build-support.py run --platform android --profile "$profile" -- bash "$0" "${arguments[@]}"
fi
# Fail before native compilation if the product version or CI release sequence is invalid.
python3 scripts/version.py --format build-info >/dev/null
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
  arguments=(--profile "$profile")
  [[ "$diagnostics" == false ]] || arguments+=(--diagnostics)
  bash scripts/build-android-native.sh "${arguments[@]}"
else
  # Skipping compilation is explicit, but a native library from another build
  # cannot silently be paired with the currently generated Kotlin interface.
  python3 scripts/build-support.py verify-bindings \
    --library "target/cross-rust/aarch64-linux-android/$output_profile/libvelune_bindings.so" \
    --language kotlin --out-dir target/android-bindings
fi
for artifact in target/android-bindings/app/velune/bindings/velune_bindings.kt target/android-native/arm64-v8a/libvelune_bindings.so target/android-native/rustls-platform-verifier-0.2.0.aar target/android-licenses/rust-licenses.txt; do
  [[ -f "$artifact" ]] || { echo "缺少构建产物：$artifact；请先运行 scripts/build-android-native.sh。" >&2; exit 1; }
done
app/android/gradlew -p app/android ":app:assemble$variant" ":app:lint$variant" --console=plain --max-workers="${VELUNE_BUILD_JOBS:-2}"
[[ -s "app/android/app/build/outputs/apk/$apk" ]] || { echo "缺少当前变体 APK：app/android/app/build/outputs/apk/$apk" >&2; exit 1; }
printf '已构建：%s/app/android/app/build/outputs/apk/%s\n' "$PWD" "$apk"
