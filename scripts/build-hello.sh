#!/usr/bin/env bash
# Build testapps/hello (no Gradle) → testapps/hello/build/hello.apk
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP="$ROOT/testapps/hello"
OUT="$APP/build"
ANDROID_HOME="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
BT="$(ls -d "$ANDROID_HOME"/build-tools/*/ 2>/dev/null | sort -V | tail -1)"
PLATFORM="$(ls -d "$ANDROID_HOME"/platforms/android-*/ 2>/dev/null | sort -V | tail -1)"
ANDROID_JAR="${PLATFORM}android.jar"
JAVA_HOME="${JAVA_HOME:-}"
if [[ -z "$JAVA_HOME" && -d "/opt/homebrew/opt/openjdk@17/libexec/openjdk.jdk/Contents/Home" ]]; then
  JAVA_HOME="/opt/homebrew/opt/openjdk@17/libexec/openjdk.jdk/Contents/Home"
fi
if [[ -z "$JAVA_HOME" && -d "/Applications/Android Studio.app/Contents/jbr/Contents/Home" ]]; then
  JAVA_HOME="/Applications/Android Studio.app/Contents/jbr/Contents/Home"
fi
export PATH="${JAVA_HOME:+$JAVA_HOME/bin:}${BT}:$PATH"

if [[ -z "$BT" || ! -f "$ANDROID_JAR" ]]; then
  echo "error: need ANDROID_HOME with build-tools + platforms" >&2
  exit 1
fi

echo "build-tools=$BT"
echo "platform=$PLATFORM"
rm -rf "$OUT"
mkdir -p "$OUT"/{gen,obj,dex,apk}

"$BT/aapt2" compile --dir "$APP/res" -o "$OUT/compiled.zip"
"$BT/aapt2" link -o "$OUT/apk/unsigned.apk" \
  -I "$ANDROID_JAR" \
  --manifest "$APP/AndroidManifest.xml" \
  --java "$OUT/gen" \
  "$OUT/compiled.zip"

javac --release 17 -classpath "$ANDROID_JAR" \
  -d "$OUT/obj" \
  "$APP/src/com/example/hello/MainActivity.java"

"$BT/d8" --min-api 26 --output "$OUT/dex" "$OUT/obj"/com/example/hello/*.class
(
  cd "$OUT/dex"
  zip -q "$OUT/apk/unsigned.apk" classes.dex
)

KS="$OUT/debug.keystore"
if [[ ! -f "$KS" ]]; then
  keytool -genkeypair -v -keystore "$KS" -storepass android -keypass android \
    -alias androiddebugkey -keyalg RSA -keysize 2048 -validity 10000 \
    -dname "CN=Android Debug,O=Android,C=US" >/dev/null 2>&1
fi
"$BT/zipalign" -f -p 4 "$OUT/apk/unsigned.apk" "$OUT/apk/aligned.apk"
"$BT/apksigner" sign --ks "$KS" --ks-pass pass:android --key-pass pass:android \
  --out "$OUT/hello.apk" "$OUT/apk/aligned.apk"
echo "OK $OUT/hello.apk"
ls -la "$OUT/hello.apk"
