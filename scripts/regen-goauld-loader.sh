#!/usr/bin/env bash
# Regenerate crates/apk-patch-core/assets/goauld_loader.dex from LoaderProvider.java
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/crates/apk-patch-core/assets/goauld_loader"
OUT_DEX="$ROOT/crates/apk-patch-core/assets/goauld_loader.dex"
ANDROID_HOME="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
BT="$(ls -d "$ANDROID_HOME"/build-tools/*/ 2>/dev/null | sort -V | tail -1)"
PLATFORM="$(ls -d "$ANDROID_HOME"/platforms/android-*/ 2>/dev/null | sort -V | tail -1)"
ANDROID_JAR="${PLATFORM}android.jar"
JAVA_HOME="${JAVA_HOME:-}"
if [[ -z "$JAVA_HOME" && -d "/opt/homebrew/opt/openjdk@17/libexec/openjdk.jdk/Contents/Home" ]]; then
  JAVA_HOME="/opt/homebrew/opt/openjdk@17/libexec/openjdk.jdk/Contents/Home"
fi
export PATH="${JAVA_HOME:+$JAVA_HOME/bin:}${BT}:$PATH"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP/classes/goauld/inject" "$TMP/dex"
cp "$SRC/LoaderProvider.java" "$TMP/classes/goauld/inject/"

javac --release 11 -classpath "$ANDROID_JAR" -d "$TMP/classes" \
  "$TMP/classes/goauld/inject/LoaderProvider.java"
"$BT/d8" --lib "$ANDROID_JAR" --min-api 21 --output "$TMP/dex" \
  "$TMP/classes/goauld/inject/LoaderProvider.class"
cp "$TMP/dex/classes.dex" "$OUT_DEX"
echo "OK $OUT_DEX ($(wc -c < "$OUT_DEX") bytes)"
