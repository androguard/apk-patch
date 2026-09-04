#!/usr/bin/env bash
# Decode → build roundtrip smoke for an APK (default: testapps/hello).
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

WORK="$(mktemp -d "${TMPDIR:-/tmp}/apk-patch-roundtrip.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

echo "==> decode $APK"
cargo run -q -p apk-patch-cli -- decode "$APK" -o "$WORK/proj" -f -s
echo "==> build"
cargo run -q -p apk-patch-cli -- build "$WORK/proj" -f -o "$WORK/out.apk"
echo "==> verify"
unzip -l "$WORK/out.apk" | head -20
echo "OK roundtrip → $WORK/out.apk ($(wc -c < "$WORK/out.apk") bytes)"
