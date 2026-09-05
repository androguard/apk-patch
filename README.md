# apk-patch

Pure Rust APK pack/unpack/patch tool — functional equivalent of [Apktool](https://github.com/iBotPeaches/Apktool), built on the [Androguard](https://github.com/androguard) crate suite.

Code is stored as **[dex-txt](./docs/DEX-TXT.md)** (not smali). Builds produce a **signed** APK by default (v1 + v2 + v3). Resources rebuild with a **pure-Rust ARSC builder**; aapt2 is optional.

## Library: in-memory / WASM (`apk-patch-vfs`)

CLI Path APIs are unchanged. Additive APIs for browsers and embedding:

- `apk-patch-vfs` — `Vfs` trait, `StdFs` (native), `MemVfs` (in-memory)
- `decode_apk_vfs` / `build_project_vfs` — VFS-backed decode/build
- `decode_apk_bytes` / `build_project_bytes` / `inject_goauld_bytes` — MemVfs convenience

Cargo features on `apk-patch-core` (defaults preserve CLI behavior):

| Feature | Default | Notes |
|---------|---------|--------|
| `parallel` | on | rayon in dex-txt; disable for WASM |
| `aapt2` | on | host `aapt2` spawn; disable for WASM |
| `native-fs` | on | `StdFs` / walkdir |
| `goauld` | on | host agent path helpers |

WASM / browser consumers should depend with `default-features = false`. Signing uses an embedded debug key on `wasm32` (no `ring`/`rcgen`).

## Install / build

From this repo (path-deps expect sibling Androguard crates under `../`):

```bash
cargo run -p apk-patch-cli -- --help
```

Optional release binary: `cargo build -p apk-patch-cli --release` → `./target/release/apk-patch`.

Global flags: `-q` / `--quiet`, `-v` / `--verbose`.

## Quick start

```bash
# Decode APK → project directory
cargo run -p apk-patch-cli -- decode app.apk -f -o out/

# Edit dex-txt under out/dex/, AndroidManifest.xml, res/, …
# Instructions are mnemonic-first (hex after # is optional commentary).
# Add/remove a class by adding/deleting a `.dex.txt` file; same for methods/fields.

# Rebuild → signed APK at out/dist/<apkFileName>
cargo run -p apk-patch-cli -- build out/ -f

# Unsigned output (Apktool-like)
cargo run -p apk-patch-cli -- build out/ -f --no-sign
```

Aliases: `d` = `decode`, `b` = `build`.

---

## Commands

| Command | Alias | Purpose |
|---------|-------|---------|
| `decode` | `d` | APK → editable project |
| `build` | `b` | Project → APK (signed by default) |
| `inject-goauld` | | Pack `libgoauld_agent.so` + early-load provider, rebuild & sign |
| `install-framework` | `if` | Install a framework APK into the cache |
| `clean-frameworks` | `cf` | Remove cached frameworks |
| `list-frameworks` | `lf` | List cached frameworks |
| `publicize-resources` | `pr` | Set `SPEC_PUBLIC` on entries in a `resources.arsc` |

---

## Decode

```bash
cargo run -p apk-patch-cli -- d app.apk -o out/ -f
```

### Common flags

| Flag | Description |
|------|-------------|
| `-f`, `--force` | Overwrite existing output directory |
| `-o`, `--output` | Output project directory |
| `-s`, `--no-src` | Skip dex-txt; copy raw `.dex` |
| `-r`, `--no-res` | Skip resource decode; keep raw `resources.arsc` |
| `-a`, `--all-src` | Disassemble all root `.dex` files (not only `classes.dex`) |
| `-j`, `--jobs` | Parallel dex-txt workers |
| `-p`, `--frame-path` | Framework cache directory |
| `-t`, `--frame-tag` | Framework tag filter |
| `--no-assets` | Skip `assets/` |
| `--only-manifest` | Decode only `AndroidManifest.xml` |
| `--no-debug-info` | Omit debug annotations in dex-txt |
| `--keep-broken-res` | Keep going if resource decode fails |
| `--res-resolve-mode` | `default` \| `greedy` \| `lazy` (how `@id` refs are rewritten) |
| `--ignore-raw-values` | Skip opaque raw Res_value dumps in values XML |

### Examples

```bash
# Manifest only
cargo run -p apk-patch-cli -- d app.apk -o man/ -f --only-manifest

# Raw dex + raw resources (fast inspect)
cargo run -p apk-patch-cli -- d app.apk -o raw/ -f -s -r

# All dex files as dex-txt
cargo run -p apk-patch-cli -- d app.apk -o full/ -f -a
```

---

## Build

```bash
cargo run -p apk-patch-cli -- b out/ -f
# → out/dist/<apkFileName>  (from apkpatch.yml)
```

Default resource path: **pure-Rust ARSC builder** from `res/values*/` + `public.xml` + file resources. Text `res/**/*.xml` is re-encoded to binary AXML. Manifest is encoded from the project-root text XML.

### Common flags

| Flag | Description |
|------|-------------|
| `-f`, `--force` | Force rebuild even if the output APK looks up-to-date |
| `-o`, `--output` | Output APK path (default: `dist/<apkFileName>`) |
| `-j`, `--jobs` | Parallel dex-txt assembly |
| `-p`, `--frame-path` | Framework directory (aapt2 `-I` / id=1 fallback) |
| `-t`, `--frame-tag` | Framework tag |
| `--debuggable` | Set `android:debuggable="true"` on `<application>` |
| `--net-sec-conf` | Inject permissive network security config |
| `--copy-original` | Pack `original/AndroidManifest.xml` + `original/META-INF/` |
| `--no-apk` | Write intermediates to `build/apk/` only (no final ZIP) |
| `--no-sign` | Leave the APK unsigned |
| `--use-aapt2` | Prefer aapt2 compile/link over the pure-Rust builder |
| `--aapt` | Path to aapt2 binary (with `--use-aapt2`) |
| `--no-crunch` | Pass `--no-crunch` to aapt2 compile |
| `--v1-signing-enabled` / `--v2-…` / `--v3-…` | Toggle signing schemes (default: all on) |

SDK / version from `apkpatch.yml` (`sdkInfo`, `versionInfo`) are applied to the text manifest on build.

### Examples

```bash
# Debug + cleartext network config
cargo run -p apk-patch-cli -- b out/ -f --debuggable --net-sec-conf

# Intermediates only
cargo run -p apk-patch-cli -- b out/ -f --no-apk
# → out/build/apk/{AndroidManifest.xml,resources.arsc,classes.dex,…}

# Optional aapt2 path (needs SDK frameworks / android.jar)
cargo run -p apk-patch-cli -- b out/ -f --use-aapt2 --aapt "$ANDROID_HOME/build-tools/36.1.0/aapt2"

# Incremental: second build with no source changes is a no-op unless -f
cargo run -p apk-patch-cli -- b out/
cargo run -p apk-patch-cli -- b out/ -f   # force
```

---

## Project layout

After `decode`:

```
out/
├── apkpatch.yml              # metadata (sdk/version, framework ids, …)
├── AndroidManifest.xml      # text XML (edit this)
├── dex/                     # classes.dex as *.dex.txt
├── dex_classes2/            # multi-dex (if present)
├── res/
│   ├── values/
│   │   ├── public.xml
│   │   ├── strings.xml
│   │   └── …
│   ├── layout/              # decoded text XML
│   └── drawable/
├── assets/
├── lib/
├── original/                # binary originals for rebuild fallback
│   ├── AndroidManifest.xml
│   ├── resources.arsc
│   └── META-INF/
└── unknown/
```

After `build`:

```
out/
├── build/
│   ├── AndroidManifest.xml  # patched text used for encode
│   ├── resources.arsc       # pure-Rust builder output (when used)
│   ├── resources.zip        # aapt2 compile output (if --use-aapt2)
│   └── apk/                 # packed entry tree (--no-apk stops here)
└── dist/
    └── app.apk              # signed APK
```

---

## Inject goauld (arm64)

One-shot: decode → copy `libgoauld_agent.so` into `lib/arm64-v8a/` → add a tiny ContentProvider DEX that calls `System.loadLibrary("goauld_agent")` at process start → rebuild & sign.

The agent starts from its ELF constructor once the `.so` is loaded. **arm64-v8a only** (matches `arm_goauld` android-arm64 dist).

```bash
# Default agent: $GOAULD_AGENT_SO or ../arm_goauld/dist/android-arm64/libgoauld_agent.so
cargo run -p apk-patch-cli -- inject-goauld app.apk -o app-goauld.apk

cargo run -p apk-patch-cli -- inject-goauld app.apk \
  --agent /path/to/libgoauld_agent.so \
  -o app-goauld.apk \
  --keep-project /tmp/app-goauld-project
```

Smoke against the bundled hello APK (needs `adb` + agent `.so`):

```bash
./scripts/build-hello.sh
./scripts/smoke-inject-goauld.sh
```

| Flag | Description |
|------|-------------|
| `-o`, `--output` | Output APK (default: `<stem>-goauld.apk` beside the input) |
| `--agent` | Path to `libgoauld_agent.so` |
| `--keep-project DIR` | Keep the decoded/injected project at `DIR` |
| `-f`, `--force` | Overwrite keep-project directory if it exists |
| `--no-sign` / `--v1-…` / `--v2-…` / `--v3-…` | Same as `build` |

### Regenerating the loader DEX

Source: `crates/apk-patch-core/assets/goauld_loader/LoaderProvider.java`. Binary: `crates/apk-patch-core/assets/goauld_loader.dex`.

```bash
./scripts/regen-goauld-loader.sh
```

Or manually:

```bash
ANDROID_JAR="$ANDROID_HOME/platforms/android-36/android.jar"
D8="$ANDROID_HOME/build-tools/36.1.0/d8"
javac --release 11 -classpath "$ANDROID_JAR" -d /tmp/gl/classes \
  crates/apk-patch-core/assets/goauld_loader/LoaderProvider.java
# (place under goauld/inject/ package dirs first)
"$D8" --lib "$ANDROID_JAR" --min-api 21 --output /tmp/gl/dex \
  /tmp/gl/classes/goauld/inject/LoaderProvider.class
cp /tmp/gl/dex/classes.dex crates/apk-patch-core/assets/goauld_loader.dex
```

---

## Scripts & testapps

| Path | Purpose |
|------|---------|
| `scripts/build-hello.sh` | Build `testapps/hello` → `hello.apk` |
| `scripts/smoke-roundtrip.sh` | Decode/build roundtrip |
| `scripts/smoke-inject-goauld.sh` | Inject + install + check `goauld` logcat |
| `scripts/regen-goauld-loader.sh` | Rebuild embedded loader DEX |
| `testapps/hello/` | Minimal Android app for smoke tests |

---

## Frameworks

Same idea as Apktool: package id `1` is the Android framework. An **embedded minimal `1.apk`** is written automatically if missing.

```bash
cargo run -p apk-patch-cli -- if framework-res.apk -p ~/.local/share/apktool/framework
cargo run -p apk-patch-cli -- lf -p ~/.local/share/apktool/framework
cargo run -p apk-patch-cli -- cf -p ~/.local/share/apktool/framework --all

# Publicize a standalone resources.arsc
cargo run -p apk-patch-cli -- pr path/to/resources.arsc
```

Default framework dirs (when `-p` omitted): macOS `~/Library/apktool/framework`, Linux XDG data, Windows LocalAppData.

---

## Editing tips

1. **DEX** — edit `dex/**/*.dex.txt`. Hex insn lines are authoritative for patching; see [DEX-TXT.md](./docs/DEX-TXT.md).
2. **Manifest** — edit root `AndroidManifest.xml` (text). Build encodes it to binary AXML.
3. **Resources** — edit `res/values*/*.xml` and layouts; keep `public.xml` ids stable when possible. Build rebuilds `resources.arsc` in Rust by default.
4. **Fallback** — if rebuild fails, the tool packs `original/resources.arsc` and warns.

---

## Differences from Apktool

| | Apktool | apk-patch |
|---|---------|-----------|
| Language | Java | Rust |
| Code format | smali | [dex-txt](./docs/DEX-TXT.md) |
| Build output | unsigned | **signed by default** |
| Resources rebuild | aapt2 | **pure-Rust ARSC** (aapt2 via `--use-aapt2`) |

---

## Documentation

- [Implementation plan](./docs/PLAN.md)
- [DEX text format](./docs/DEX-TXT.md)
- [Apktool parity checklist](./docs/PARITY.md)

## License

Apache-2.0 — see [LICENSE](./LICENSE).
