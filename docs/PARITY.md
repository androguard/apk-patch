# Apktool Parity Checklist

Tracking apk-patch progress against [Apktool 3.x](https://github.com/iBotPeaches/Apktool) (`main` branch).

**Legend:** ☐ not started · ◐ in progress · ☑ done · ➖ N/A (intentional difference)

**Parity tests:** `crates/apk-patch-core/tests/apktool_parity.rs` mirrors Apktool test names (`BuildAndDecodeApkTest`, `FrameworkTest`, `SkipAssetTest`, …).

---

## Intentional Differences

| Feature | Apktool | apk-patch |
|---------|---------|-----------|
| Code format | smali | dex-txt |
| Code directories | `smali/` | `dex/` |
| Build signing | unsigned (manual) | **signed by default** |
| `--no-sign` flag | N/A | outputs unsigned APK |

---

## CLI Surface

### Commands

| Feature | Status |
|---------|--------|
| `decode` / `d` | ☑ |
| `build` / `b` | ☑ |
| `install-framework` / `if` | ☑ |
| `clean-frameworks` / `cf` | ☑ |
| `list-frameworks` / `lf` | ☑ |
| `publicize-resources` / `pr` | ☑ |
| `help` / `h` | ☑ |
| `version` / `v` | ☑ |
| Global `-q` / `--quiet` | ☑ |
| Global `-v` / `--verbose` | ☑ |
| Short-flag chaining (3.x) | ☐ |

### Decode flags

| Flag | Status |
|------|--------|
| `-f` | ☑ |
| `-o` / `--output` | ☑ |
| `-r` / `--no-res` | ☑ |
| `-s` / `--no-src` | ☑ |
| `-a` / `--all-src` | ☑ |
| `-j` / `--jobs` | ☑ |
| `-p` / `--frame-path` | ☑ |
| `-t` / `--frame-tag` | ☑ |
| `-l` / `--lib` | ☐ |
| `--no-debug-info` | ☑ |
| `--keep-broken-res` | ☑ |
| `--match-original` | ☐ |
| `--no-assets` | ☑ |
| `--only-manifest` | ☑ |
| `--res-resolve-mode` | ☑ |
| `--ignore-raw-values` | ☑ |
| Flag conflict validation | ☐ |

### Build flags

| Flag | Status |
|------|--------|
| `-f` | ☑ |
| `-o` / `--output` | ☑ |
| `-j` / `--jobs` | ☑ |
| `-p` / `--frame-path` | ☐ |
| `-l` / `--lib` | ☐ |
| `--aapt` | ☑ |
| `--copy-original` | ☑ |
| `--debuggable` | ☑ |
| `--net-sec-conf` | ☑ |
| `--no-apk` | ☑ |
| `--no-crunch` | ☑ |

### Signing flags (apk-patch extension)

| Flag | Status |
|------|--------|
| `--no-sign` | ☑ |
| `--ks` | ☐ |
| `--ks-pass` | ☐ |
| `--ks-key-alias` | ☐ |
| `--key-pass` | ☐ |
| `--v1-signing-enabled` | ☑ |
| `--v2-signing-enabled` | ☑ |
| `--v3-signing-enabled` | ☑ |

---

## Decode Pipeline

| Feature | Status |
|---------|--------|
| APK read with ZIP hardening | ◐ |
| Output dir existence check + `-f` overwrite | ☑ |
| Parallel jobs (`-j`) | ☑ |
| dex-txt disassembly (default) | ☑ |
| Raw dex copy (`-s`) | ☑ |
| All-dex disassembly (`-a`) | ☑ |
| Resource decode (full) | ◐ values*/overlayable + binary XML; ARSC encode pending |
| Resource skip (`-r`) | ☑ |
| Manifest-only decode | ☑ |
| Asset copy / skip | ☑ |
| `--match-original` analysis mode | ☐ |
| `--keep-broken-res` tolerance | ☑ |
| `--res-resolve-mode` + dummy resources | ◐ modes wired; dummy resources TBD |
| `--ignore-raw-values` | ☑ |
| Multi-dex directory naming (`dex_classes2/`, etc.) | ☑ |
| Odex rejection with clear error | ☑ |
| API level inference from opcodes | ☐ |

---

## Output Layout

| Feature | Status |
|---------|--------|
| `apktool.yml` | ☑ |
| `AndroidManifest.xml` (text) | ☑ |
| `dex/` and `dex_*` directories | ☑ |
| `res/` tree | ◐ values* + decoded XML + binary assets |
| `assets/` | ☑ |
| `lib/` | ☑ |
| `original/` (manifest + META-INF + arsc) | ☑ |
| `unknown/` | ☑ |
| Raw dex when `-s` | ☑ |
| Raw `resources.arsc` when `-r` | ☑ |

---

## Build Pipeline

| Feature | Status |
|---------|--------|
| Load and validate `apktool.yml` | ☑ |
| Incremental rebuild (mtime checks) | ☐ |
| Force rebuild (`-f`) | ☑ |
| dex-txt → dex assembly | ☑ |
| Raw dex passthrough | ☑ |
| aapt2 compile + link | ☑ optional (`--use-aapt2`); pure-Rust default |
| Manifest backup/restore (`.orig`) | ☐ |
| Provider authority fixups | ☐ |
| `build/apk/` intermediate output | ☑ |
| `build/resources.zip` | ☑ |
| `dist/{apkFileName}` final output | ☑ |
| `--no-apk` intermediate-only mode | ☑ |
| zipalign | ☐ |
| Automatic signing (v1 + v2 + v3) | ☑ |

---

## Resources

| Feature | Status |
|---------|--------|
| Full ARSC parse | ☑ configs, sparse/compact, value types, overlayable |
| Full ARSC encode | ☑ pure-Rust builder from public.xml + values* + file res |
| Framework-dependent reference resolution | ◐ embedded id=1; full attr resolve TBD |
| `public.xml` generation | ☑ |
| Values XML per type + qualifier | ☑ |
| Layout / drawable / menu decode | ☑ text via AXMLPrinter |
| 9-patch decode | ☑ `npTc` parse helpers |
| `overlayable.xml` | ☑ |
| `sparseEntries`, `compactEntries`, `keepRawValues` | ◐ parse + `--ignore-raw-values` |
| `featureFlags` | ☐ |
| Non-standard resource directories | ☐ |

---

## Framework System

| Feature | Status |
|---------|--------|
| Default framework paths (macOS / Linux / Windows) | ☑ |
| `if`: install + publicize + tag | ☑ |
| `cf`: clean with `-a`, `-t` | ☑ |
| `lf`: list with `-a`, `-t` | ☑ |
| Embedded android framework (id=1) fallback | ☑ |
| `usesFramework.ids` + `tag` in yml | ☑ |
| `pr` / publicize-resources | ☑ |

---

## Metadata (`apktool.yml`)

| Feature | Status |
|---------|--------|
| Read/write 3.x schema | ☑ |
| 2.x field migration | ☑ |
| `doNotCompress` (extensions + full paths) | ◐ defaults |
| SDK codename parsing | ☐ |
| `usesLibrary` + CLI `--lib` | ☐ |

---

## Manifest

| Feature | Status |
|---------|--------|
| Binary → text AXML | ☑ |
| Text → binary AXML | ☑ |
| Extract/write sdkInfo, versionInfo, featureFlags | ◐ sdk/version |
| Strip/reapply versionCode/versionName via aapt2 | ☑ yml + `--replace-version` |
| `--only-manifest` decode path | ☑ |
| `--debuggable` build injection | ☑ |
| `--net-sec-conf` build injection | ☑ |

---

## Compatibility

| Feature | Status |
|---------|--------|
| 2.x decoded project import | ◐ yml migrate |
| Log prefix format (`I:`/`W:`/`E:`/`D:`) | ☑ |
| Framework-not-found actionable error | ☑ |
| Split APK / APKM workflows | ☐ |

---

## Non-Goals

| Feature | Status |
|---------|--------|
| Java/Kotlin decompilation | ➖ use JADX |
| Odex/deodex | ➖ |
| aapt1 build path | ➖ |
| Smali compatibility | ➖ use dex-txt |
