<p align="center">
  <img src="./.github/banner.jpg" alt="apk-patch — pack / unpack / patch" width="200">
</p>

# apk-patch

Pure Rust APK pack / unpack / patch tool — Apktool-like workflow on the
[Androguard](https://github.com/androguard) crate suite.

- **Code format:** [dex-txt](./docs/DEX-TXT.md) (not smali)
- **Build output:** signed APK by default (v1 + v2 + v3)
- **Resources:** pure-Rust `resources.arsc` rebuild (optional aapt2)
- **Containers:** `.apk`, `.xapk`, and `.apkm` round-trip

---

## How to use (CLI)

### Build the binary

Androguard dependencies (`apkparser`, `axml-parser`, `dex-parser`, `dex-bytecode`)
are pulled from GitHub — no sibling checkouts required.

```bash
cargo build -p apk-patch-cli --release
./target/release/apk-patch --help

# or without installing:
cargo run -p apk-patch-cli -- --help
```

Global flags: `-q` / `--quiet`, `-v` / `--verbose`.

### Decode → edit → build

```bash
# 1. Decode APK (or XAPK / APKM) into an editable project
apk-patch decode app.apk -f -o out/

# 2. Edit what you need:
#    - out/dex/**/*.dex.txt     (bytecode)
#    - out/AndroidManifest.xml (text)
#    - out/res/**              (resources)
#    - out/apkpatch.yml        (sdk/version metadata)

# 3. Rebuild → signed package at out/dist/<apkFileName>
apk-patch build out/ -f
```

Aliases: `d` = `decode`, `b` = `build`.

```bash
# Unsigned (Apktool-like)
apk-patch build out/ -f --no-sign

# XAPK / APKM: decode preserves splits under container/; build repacks .xapk/.apkm
apk-patch decode Facebook.xapk -f -o out-fb/
apk-patch build out-fb/ -f
# → out-fb/dist/Facebook.xapk  (rebuilt base + original splits)
```

### Typical decode variants

```bash
# Manifest only
apk-patch d app.apk -o man/ -f --only-manifest

# Fast inspect: raw .dex + raw resources.arsc
apk-patch d app.apk -o raw/ -f -s -r

# Disassemble every classes*.dex as dex-txt
apk-patch d app.apk -o full/ -f -a
```

### Typical build variants

```bash
# Debuggable + cleartext traffic config
apk-patch b out/ -f --debuggable --net-sec-conf

# Write intermediates only (no final ZIP)
apk-patch b out/ -f --no-apk
# → out/build/apk/{AndroidManifest.xml,resources.arsc,classes.dex,…}

# Optional aapt2 resource path
apk-patch b out/ -f --use-aapt2 --aapt "$ANDROID_HOME/build-tools/36.1.0/aapt2"
```

### Inject goauld (arm64)

One-shot: decode → pack `libgoauld_agent.so` + early-load ContentProvider → rebuild & sign.

```bash
# Default agent: $GOAULD_AGENT_SO or ../arm_goauld/dist/android-arm64/libgoauld_agent.so
apk-patch inject-goauld app.apk -o app-goauld.apk

apk-patch inject-goauld app.apk \
  --agent /path/to/libgoauld_agent.so \
  -o app-goauld.apk \
  --keep-project /tmp/app-goauld-project
```

Smoke (needs `adb` + agent `.so`):

```bash
./scripts/build-hello.sh
./scripts/smoke-inject-goauld.sh
```

---

## How to use (Python)

In-memory bindings live in `crates/apk-patch-py` (module name `apk_patch`).
They decode / build from **bytes** — no temp project on disk required.

### Install

Requires a Rust toolchain and [maturin](https://www.maturin.rs/):

```bash
pip install maturin
cd crates/apk-patch-py
maturin develop --release
# or: maturin build --release  → wheel under target/wheels/
```

### API

| Function | Role |
|----------|------|
| `apk_patch.decode(apk_bytes, …)` | APK → in-memory project (`files` dict) |
| `apk_patch.build(files, …)` | Project dict → APK bytes |
| `apk_patch.inject_goauld(apk_bytes, agent_so, …)` | Inject arm64 agent + rebuild |

`decode` returns:

```python
{
  "files": dict[str, bytes],   # paths like "project/AndroidManifest.xml"
  "project_root": str,         # usually "project"
  "entry_count": int,
  "dex_class_count": int,
}
```

### Examples

**Round-trip (decode → tweak → build):**

```python
from pathlib import Path
import apk_patch

apk = Path("app.apk").read_bytes()
proj = apk_patch.decode(apk, apk_name="app.apk")

files = proj["files"]
root = proj["project_root"]  # "project"

# Edit the text manifest in memory
manifest_key = f"{root}/AndroidManifest.xml"
xml = files[manifest_key].decode("utf-8")
xml = xml.replace('android:debuggable="false"', 'android:debuggable="true"')
files[manifest_key] = xml.encode("utf-8")

# Rebuild (signed by default)
out = apk_patch.build(files, project_root=root, sign=True)
Path("app-patched.apk").write_bytes(out)
```

**Skip sources / resources (faster inspect):**

```python
proj = apk_patch.decode(
    apk,
    apk_name="app.apk",
    no_src=True,       # keep raw .dex, no dex-txt
    no_res=True,       # keep raw resources.arsc
)
# or: only_manifest=True
```

**Inject goauld agent:**

```python
agent = Path("libgoauld_agent.so").read_bytes()
out = apk_patch.inject_goauld(apk, agent, apk_name="app.apk", sign=True)
Path("app-goauld.apk").write_bytes(out)
```

**List / patch a dex-txt class:**

```python
proj = apk_patch.decode(apk, all_src=True)
files = proj["files"]
root = proj["project_root"]

# Find a class file
keys = [k for k in files if k.endswith(".dex.txt")]
sample = keys[0]
text = files[sample].decode("utf-8")
# …edit mnemonic lines…
files[sample] = text.encode("utf-8")

out = apk_patch.build(files, project_root=root)
```

Notes:

- Paths inside `files` must stay under `project_root` (same layout as a CLI decode).
- Python build uses the pure-Rust ARSC path (`rebuild_resources=True`); it does not spawn aapt2.
- Signing uses the same debug keystore as the CLI unless you pass `sign=False`.

---

## Project layout

After `decode`:

```
out/
├── apkpatch.yml             # metadata (sdk/version, packageFormat, …)
├── AndroidManifest.xml      # text XML (edit this)
├── dex/                     # classes.dex → *.dex.txt
├── dex_classes2/            # multi-dex (if present)
├── res/
├── assets/
├── lib/
├── container/               # XAPK/APKM splits + manifest.json (if any)
├── original/                # binary originals for rebuild fallback
└── unknown/
```

After `build`:

```
out/
├── build/apk/               # packed entry tree (--no-apk stops here)
└── dist/
    └── app.apk              # or .xapk / .apkm when decoded from a container
```

---

## Commands reference

| Command | Alias | Purpose |
|---------|-------|---------|
| `decode` | `d` | APK / XAPK / APKM → editable project |
| `build` | `b` | Project → APK (or XAPK/APKM); signed by default |
| `inject-goauld` | | Pack `libgoauld_agent.so` + early-load provider |
| `install-framework` | `if` | Install a framework APK into the cache |
| `clean-frameworks` | `cf` | Remove cached frameworks |
| `list-frameworks` | `lf` | List cached frameworks |
| `publicize-resources` | `pr` | Set `SPEC_PUBLIC` on entries in a `resources.arsc` |

### Decode flags

| Flag | Description |
|------|-------------|
| `-f`, `--force` | Overwrite existing output directory |
| `-o`, `--output` | Output project directory |
| `-s`, `--no-src` | Skip dex-txt; copy raw `.dex` |
| `-r`, `--no-res` | Skip resource decode; keep raw `resources.arsc` |
| `-a`, `--all-src` | Disassemble all root `.dex` files |
| `-j`, `--jobs` | Parallel dex-txt workers |
| `-p` / `-t` | Framework path / tag |
| `--no-assets` | Skip `assets/` |
| `--only-manifest` | Decode only `AndroidManifest.xml` |
| `--no-debug-info` | Omit debug annotations in dex-txt |
| `--keep-broken-res` | Keep going if resource decode fails |
| `--res-resolve-mode` | `default` \| `greedy` \| `lazy` |
| `--ignore-raw-values` | Skip opaque raw Res_value dumps |

### Build flags

| Flag | Description |
|------|-------------|
| `-f`, `--force` | Force rebuild even if output looks up-to-date |
| `-o`, `--output` | Output path (default: `dist/<apkFileName>`) |
| `-j`, `--jobs` | Parallel dex-txt assembly |
| `--debuggable` | Set `android:debuggable="true"` |
| `--net-sec-conf` | Inject permissive network security config |
| `--copy-original` | Pack `original/` manifest + META-INF |
| `--no-apk` | Intermediates only (`build/apk/`) |
| `--no-sign` | Leave unsigned |
| `--use-aapt2` / `--aapt` / `--no-crunch` | Optional aapt2 resource path |
| `--v1-signing-enabled` / `--v2-…` / `--v3-…` | Toggle signing schemes |

SDK / version from `apkpatch.yml` are applied to the text manifest on build.
Legacy `apktool.yml` is still accepted on load.

### Frameworks

```bash
apk-patch if framework-res.apk -p ~/.local/share/apktool/framework
apk-patch lf -p ~/.local/share/apktool/framework
apk-patch cf -p ~/.local/share/apktool/framework --all
apk-patch pr path/to/resources.arsc
```

Default framework dirs (when `-p` omitted): macOS `~/Library/apktool/framework`, Linux XDG data, Windows LocalAppData. An embedded minimal `1.apk` is written automatically if missing.

---

## Library (Rust / WASM)

Additive APIs for embedding and browsers:

- `apk-patch-vfs` — `Vfs`, `StdFs` (native), `MemVfs` (in-memory)
- `decode_apk_vfs` / `build_project_vfs`
- `decode_apk_bytes` / `build_project_bytes` / `inject_goauld_bytes`

Cargo features on `apk-patch-core` (defaults preserve CLI behavior):

| Feature | Default | Notes |
|---------|---------|--------|
| `parallel` | on | rayon in dex-txt; disable for WASM |
| `aapt2` | on | host `aapt2` spawn; disable for WASM |
| `native-fs` | on | `StdFs` / walkdir |
| `goauld` | on | host agent path helpers |

WASM / browser: depend with `default-features = false`. Signing uses an embedded debug key on `wasm32`.

---

## Editing tips

1. **DEX** — edit `dex/**/*.dex.txt`. See [DEX-TXT.md](./docs/DEX-TXT.md). Add/remove a class by adding/deleting a `.dex.txt` file.
2. **Manifest** — edit root `AndroidManifest.xml` (text). Build encodes it to binary AXML.
3. **Resources** — edit `res/values*/*.xml` and layouts; keep `public.xml` ids stable when possible.
4. **Fallback** — if resource rebuild fails, the tool packs `original/resources.arsc` and warns.

---

## Scripts & testapps

| Path | Purpose |
|------|---------|
| `scripts/build-hello.sh` | Build `testapps/hello` → `hello.apk` |
| `scripts/smoke-roundtrip.sh` | Decode/build roundtrip |
| `scripts/smoke-inject-goauld.sh` | Inject + install + check logcat |
| `scripts/regen-goauld-loader.sh` | Rebuild embedded loader DEX |
| `testapps/hello/` | Minimal Android app for smoke tests |

---

## Differences from Apktool

| | Apktool | apk-patch |
|---|---------|-----------|
| Language | Java | Rust |
| Code format | smali | [dex-txt](./docs/DEX-TXT.md) |
| Metadata file | `apktool.yml` | `apkpatch.yml` (loads legacy `apktool.yml`) |
| Build output | unsigned | **signed by default** |
| Resources rebuild | aapt2 | **pure-Rust ARSC** (aapt2 via `--use-aapt2`) |
| Split packages | limited | **XAPK / APKM** round-trip |

---

## Documentation

- [DEX text format](./docs/DEX-TXT.md)
- [Internals](./docs/INTERNALS.md)

## License

Apache-2.0 — see [LICENSE](./LICENSE).
