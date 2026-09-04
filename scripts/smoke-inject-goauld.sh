#!/usr/bin/env bash
# inject-goauld → adb install → launch → expect log tag "goauld".
#
# Usage:
#   ./scripts/smoke-inject-goauld.sh [apk]
# Env:
#   GOAULD_AGENT_SO   agent path (optional)
#   PACKAGE           override package (auto from aapt dump badging)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

APK="${1:-}"
if [[ -z "$APK" ]]; then
  if [[ ! -f testapps/hello/build/hello.apk ]]; then
    ./scripts/build-hello.sh
  fi
  APK="testapps/hello/build/hello.apk"
fi
APK="$(cd "$(dirname "$APK")" && pwd)/$(basename "$APK")"
OUT="${ROOT}/testapps/hello/build/hello-goauld.apk"
ANDROID_HOME="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
export PATH="${ANDROID_HOME}/platform-tools:${ANDROID_HOME}/build-tools/$(ls "$ANDROID_HOME/build-tools" | sort -V | tail -1):$PATH"

if ! adb get-state >/dev/null 2>&1; then
  echo "error: no adb device" >&2
  exit 1
fi

echo "==> inject-goauld"
cargo run -p apk-patch-cli -- inject-goauld "$APK" -o "$OUT" -v
echo "==> install"
adb install -r "$OUT"
PKG="${PACKAGE:-$(aapt dump badging "$OUT" 2>/dev/null | sed -n "s/^package: name='\([^']*\)'.*/\1/p" | head -1)}"
if [[ -z "$PKG" ]]; then
  PKG="com.example.hello"
fi
echo "==> package $PKG"
adb logcat -c
adb shell monkey -p "$PKG" -c android.intent.category.LAUNCHER 1 >/dev/null
sleep 2
if adb logcat -d -s goauld:D | rg -q "goauld agent constructor|listening on abstract"; then
  echo "OK agent alive"
  adb logcat -d -s goauld:D | tail -5
else
  echo "FAIL: no goauld agent log lines" >&2
  adb logcat -d -s goauld:D AndroidRuntime:E | tail -40
  exit 1
fi
