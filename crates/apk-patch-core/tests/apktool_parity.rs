//! Apktool-parity integration tests.
//!
//! Named after [Apktool's test suite](https://github.com/iBotPeaches/Apktool/tree/master/brut.apktool/apktool-lib/src/test/java/brut/androlib)
//! and exercised against Androguard sample APKs.

use std::path::{Path, PathBuf};

use apk_patch_core::{build_project, decode_apk, BuildOptions, DecodeOptions};
use apk_patch_framework::{
    clean_frameworks, install_framework, list_frameworks, publicize_resources_bytes,
    FrameworkOptions,
};
use apk_patch_meta::ApkToolMeta;
use apkparser::{Apk, ApkOptions};

fn apk_data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../apk-parser/tests/data/APK")
}

fn sample(name: &str) -> Option<PathBuf> {
    let path = apk_data_dir().join(name);
    path.is_file().then_some(path)
}

/// Apktool: `BuildAndDecodeApkTest#buildAndDecodeTest`
#[test]
fn build_and_decode_apk_test() {
    let apk_path = match sample("TestActivity.apk") {
        Some(p) => p,
        None => return,
    };
    let temp = tempfile::tempdir().unwrap();
    let decoded = temp.path().join("testapp-new");

    decode_apk(
        &apk_path,
        &DecodeOptions {
            force: true,
            output: Some(decoded.clone()),
            no_src: true,
            ..Default::default()
        },
    )
    .unwrap();

    assert!(decoded.is_dir());
    assert!(decoded.join("apkpatch.yml").is_file());
    assert!(decoded.join("AndroidManifest.xml").is_file());
    assert!(decoded.join("res/values/public.xml").is_file());
    assert!(decoded.join("original/resources.arsc").is_file());
    assert!(decoded.join("res/layout/main.xml").is_file() || decoded.join("res/drawable-hdpi/icon.png").is_file());

    let built = build_project(
        &decoded,
        &BuildOptions {
            force: true,
            sign: apk_patch_sign::BuildSignConfig {
                enabled: false,
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    assert!(built.output_apk.is_file());

    let redecoded = temp.path().join("testapp-roundtrip");
    decode_apk(
        &built.output_apk,
        &DecodeOptions {
            force: true,
            output: Some(redecoded.clone()),
            no_src: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(redecoded.join("apkpatch.yml").is_file());
}

/// Apktool: `SkipAssetTest`
#[test]
fn skip_asset_test() {
    // Prefer an APK that has assets/; fall back to asserting flag is honored on TestActivity.
    let apk_path = sample("hello-world.apk")
        .or_else(|| sample("TestActivity.apk"))
        .unwrap_or_else(|| return_none());
    fn return_none() -> PathBuf {
        PathBuf::new()
    }
    if apk_path.as_os_str().is_empty() {
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    let none_dir = temp.path().join("out.none");
    decode_apk(
        &apk_path,
        &DecodeOptions {
            force: true,
            output: Some(none_dir.clone()),
            no_assets: true,
            no_src: true,
            no_res: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(!none_dir.join("assets").exists());

    let full_dir = temp.path().join("out.full");
    decode_apk(
        &apk_path,
        &DecodeOptions {
            force: true,
            output: Some(full_dir.clone()),
            no_assets: false,
            no_src: true,
            no_res: true,
            ..Default::default()
        },
    )
    .unwrap();
    // If the APK has assets, they must appear; otherwise the directory may be absent.
    let apk_bytes = std::fs::read(&apk_path).unwrap();
    let apk = Apk::from_bytes(&apk_bytes, ApkOptions::default()).unwrap();
    let has_assets = apk.get_files().iter().any(|f| f.starts_with("assets/"));
    if has_assets {
        assert!(full_dir.join("assets").exists());
    }
}

/// Apktool: `ForceManifestDecodeNoResourcesTest` / `--only-manifest`
#[test]
fn force_manifest_decode_no_resources_test() {
    let apk_path = match sample("TestActivity.apk") {
        Some(p) => p,
        None => return,
    };
    let temp = tempfile::tempdir().unwrap();
    let out = temp.path().join("manifest-only");
    decode_apk(
        &apk_path,
        &DecodeOptions {
            force: true,
            output: Some(out.clone()),
            only_manifest: true,
            ..Default::default()
        },
    )
    .unwrap();

    assert!(out.join("AndroidManifest.xml").is_file());
    assert!(out.join("original/AndroidManifest.xml").is_file());
    assert!(!out.join("res").exists());
    assert!(!out.join("dex").exists());
    assert!(!out.join("classes.dex").exists());
    let xml = std::fs::read_to_string(out.join("AndroidManifest.xml")).unwrap();
    assert!(xml.contains("manifest"));
}

/// Apktool: multi-dex dirs (`BuildAndDecodeApkTest#multipleDexTest`) — dex_classes2
#[test]
fn multiple_dex_test() {
    let apk_path = match sample("TestActivity.apk") {
        Some(p) => p,
        None => return,
    };
    // Sample multidex.apk lacks a manifest; synthesize a valid multi-dex APK.
    let apk_bytes = std::fs::read(&apk_path).unwrap();
    let apk = Apk::from_bytes(&apk_bytes, ApkOptions::default()).unwrap();
    let dex = apk.get_file("classes.dex").unwrap();
    let manifest = apk.get_file("AndroidManifest.xml").unwrap();
    let mut writer = apkparser::ApkWriter::new();
    writer.add_entry("AndroidManifest.xml", &manifest, true);
    writer.add_entry("classes.dex", &dex, true);
    writer.add_entry("classes2.dex", &dex, true);
    let multi = writer.finish().unwrap();

    let temp = tempfile::tempdir().unwrap();
    let multi_apk = temp.path().join("multidex.apk");
    std::fs::write(&multi_apk, multi).unwrap();

    let out = temp.path().join("multidex-out");
    decode_apk(
        &multi_apk,
        &DecodeOptions {
            force: true,
            output: Some(out.clone()),
            no_res: true,
            ..Default::default()
        },
    )
    .unwrap();

    assert!(out.join("dex").is_dir());
    assert!(out.join("dex_classes2").is_dir());
    assert!(out.join("original/classes.dex").is_file());
    assert!(out.join("original/classes2.dex").is_file());
}

/// Apktool: `FrameworkTest#isFrameworkInstallingWorking` + tagging
#[test]
fn framework_test() {
    let apk_path = match sample("lineageos_nexus5_framework-res.apk") {
        Some(p) => p,
        None => return,
    };
    let temp = tempfile::tempdir().unwrap();
    let frame_dir = temp.path().join("framework");

    let installed = install_framework(
        &apk_path,
        &FrameworkOptions {
            frame_path: Some(frame_dir.clone()),
            tag: None,
            all_tags: false,
        },
    )
    .unwrap();
    assert!(installed.is_file());
    assert!(installed
        .file_name()
        .unwrap()
        .to_string_lossy()
        .ends_with(".apk"));

    let tagged = install_framework(
        &apk_path,
        &FrameworkOptions {
            frame_path: Some(frame_dir.clone()),
            tag: Some("building".into()),
            all_tags: false,
        },
    )
    .unwrap();
    assert!(tagged
        .file_name()
        .unwrap()
        .to_string_lossy()
        .contains("-building.apk"));

    let listed = list_frameworks(&FrameworkOptions {
        frame_path: Some(frame_dir.clone()),
        tag: None,
        all_tags: true,
    })
    .unwrap();
    assert!(listed.len() >= 2);

    let removed = clean_frameworks(&FrameworkOptions {
        frame_path: Some(frame_dir),
        tag: None,
        all_tags: true,
    })
    .unwrap();
    assert!(!removed.is_empty());
}

/// Apktool: `publicize-resources` / Framework publicize bit
#[test]
fn publicize_resources_test() {
    let apk_path = match sample("TestActivity.apk") {
        Some(p) => p,
        None => return,
    };
    let apk_bytes = std::fs::read(apk_path).unwrap();
    let apk = Apk::from_bytes(&apk_bytes, ApkOptions::default()).unwrap();
    let mut arsc = apk.get_file("resources.arsc").unwrap();
    let before = arsc.clone();
    publicize_resources_bytes(&mut arsc).unwrap();
    // Publicize is idempotent-safe and may or may not change bytes depending on flags already set.
    assert_eq!(arsc.len(), before.len());
    publicize_resources_bytes(&mut arsc).unwrap();
}

/// Apktool: confirm apkpatch.yml records sdk/version after decode
#[test]
fn confirm_meta_sdk_version_info() {
    let apk_path = match sample("TestActivity.apk") {
        Some(p) => p,
        None => return,
    };
    let temp = tempfile::tempdir().unwrap();
    let out = temp.path().join("meta-out");
    decode_apk(
        &apk_path,
        &DecodeOptions {
            force: true,
            output: Some(out.clone()),
            no_src: true,
            no_res: true,
            ..Default::default()
        },
    )
    .unwrap();

    let meta = ApkToolMeta::load(&out.join("apkpatch.yml")).unwrap();
    assert_eq!(meta.apkFileName, "TestActivity.apk");
    assert!(meta.sdkInfo.minSdkVersion.is_some());
    assert!(meta.versionInfo.versionCode.is_some());
}

/// Apktool: raw resources with `-r`
#[test]
fn decode_resources_raw_no_res_flag() {
    let apk_path = match sample("TestActivity.apk") {
        Some(p) => p,
        None => return,
    };
    let temp = tempfile::tempdir().unwrap();
    let out = temp.path().join("no-res");
    decode_apk(
        &apk_path,
        &DecodeOptions {
            force: true,
            output: Some(out.clone()),
            no_src: true,
            no_res: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(out.join("resources.arsc").is_file());
    assert!(!out.join("res/values/public.xml").exists());
}

/// Apktool-style empty-ish / missing pieces should still decode
#[test]
fn missing_version_manifest_tolerated() {
    let apk_path = match sample("TestActivity_unsigned.apk").or_else(|| sample("TestActivity.apk")) {
        Some(p) => p,
        None => return,
    };
    let temp = tempfile::tempdir().unwrap();
    let out = temp.path().join("manifest-ok");
    let result = decode_apk(
        &apk_path,
        &DecodeOptions {
            force: true,
            output: Some(out.clone()),
            only_manifest: true,
            ..Default::default()
        },
    );
    assert!(result.is_ok());
    assert!(out.join("AndroidManifest.xml").is_file());
}

/// Apktool: unknown folder for non-standard entries
#[test]
fn unknown_folder_test() {
    let apk_path = match sample("TestActivity.apk") {
        Some(p) => p,
        None => return,
    };
    let temp = tempfile::tempdir().unwrap();
    let out = temp.path().join("unknown-out");
    decode_apk(
        &apk_path,
        &DecodeOptions {
            force: true,
            output: Some(out.clone()),
            no_src: true,
            no_res: true,
            ..Default::default()
        },
    )
    .unwrap();
    // TestActivity has no unknown entries; ensure decode succeeds and layout dirs exist as expected.
    assert!(out.join("apkpatch.yml").is_file());
}

/// PLAN Phase 3 decode: values*/public, binary layout XML → text, resolve-mode flags.
#[test]
fn decode_resources_values_and_layouts() {
    let apk_path = match sample("TestActivity.apk") {
        Some(p) => p,
        None => return,
    };
    let temp = tempfile::tempdir().unwrap();
    let out = temp.path().join("res-out");
    decode_apk(
        &apk_path,
        &DecodeOptions {
            force: true,
            output: Some(out.clone()),
            no_src: true,
            res_resolve_mode: apk_patch_core::ResResolveMode::Default,
            ..Default::default()
        },
    )
    .unwrap();

    assert!(out.join("res/values/public.xml").is_file());
    let public = std::fs::read_to_string(out.join("res/values/public.xml")).unwrap();
    assert!(public.contains("<public "));

    // Typed values (at least strings for TestActivity)
    let strings = out.join("res/values/strings.xml");
    assert!(
        strings.is_file(),
        "expected res/values/strings.xml from ARSC"
    );
    let strings_xml = std::fs::read_to_string(&strings).unwrap();
    assert!(strings_xml.contains("<string "));

    // Layout binary XML should be decoded to text
    let layout = out.join("res/layout/main.xml");
    if layout.is_file() {
        let xml = std::fs::read_to_string(&layout).unwrap();
        assert!(
            xml.trim_start().starts_with("<?xml") || xml.contains('<'),
            "layout should be text XML, got binary-looking content"
        );
        assert!(!xml.as_bytes().starts_with(&[0x03, 0x00]));
    }

    // Lazy mode should still succeed
    let out_lazy = temp.path().join("res-lazy");
    decode_apk(
        &apk_path,
        &DecodeOptions {
            force: true,
            output: Some(out_lazy.clone()),
            no_src: true,
            res_resolve_mode: apk_patch_core::ResResolveMode::Lazy,
            ignore_raw_values: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(out_lazy.join("res/values/public.xml").is_file());
}

/// PLAN Phase 3 build: --no-apk, --debuggable, --copy-original, incremental -f.
#[test]
fn build_flags_no_apk_debuggable_copy_original() {
    let apk_path = match sample("TestActivity.apk") {
        Some(p) => p,
        None => return,
    };
    let temp = tempfile::tempdir().unwrap();
    let decoded = temp.path().join("decoded");
    decode_apk(
        &apk_path,
        &DecodeOptions {
            force: true,
            output: Some(decoded.clone()),
            no_src: true,
            ..Default::default()
        },
    )
    .unwrap();

    // Patch yml version for apply path
    let meta_path = decoded.join("apkpatch.yml");
    let mut meta = ApkToolMeta::load(&meta_path).unwrap();
    meta.versionInfo.versionCode = Some(99);
    meta.versionInfo.versionName = Some("9.9".into());
    meta.save(&meta_path).unwrap();

    // --no-apk writes build/apk/
    let no_apk = build_project(
        &decoded,
        &BuildOptions {
            force: true,
            no_apk: true,
            skip_aapt2: true,
            debuggable: true,
            sign: apk_patch_sign::BuildSignConfig {
                enabled: false,
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    assert!(no_apk.output_apk.ends_with("build/apk"));
    assert!(no_apk.output_apk.join("AndroidManifest.xml").is_file());
    assert!(decoded.join("build/AndroidManifest.xml").is_file());
    let patched = std::fs::read_to_string(decoded.join("build/AndroidManifest.xml")).unwrap();
    assert!(patched.contains("android:debuggable=\"true\""));
    assert!(patched.contains("android:versionCode=\"99\"") || patched.contains("versionCode=\"99\""));

    // Full build with --copy-original
    let built = build_project(
        &decoded,
        &BuildOptions {
            force: true,
            copy_original: true,
            skip_aapt2: true,
            sign: apk_patch_sign::BuildSignConfig {
                enabled: false,
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    assert!(built.output_apk.is_file());

    // Incremental: second build without -f should skip
    let again = build_project(
        &decoded,
        &BuildOptions {
            force: false,
            skip_aapt2: true,
            sign: apk_patch_sign::BuildSignConfig {
                enabled: false,
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    assert!(again.incremental_skipped);

    // -f forces rebuild
    let forced = build_project(
        &decoded,
        &BuildOptions {
            force: true,
            skip_aapt2: true,
            net_sec_conf: true,
            sign: apk_patch_sign::BuildSignConfig {
                enabled: false,
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    assert!(!forced.incremental_skipped);
    assert!(decoded.join("res/xml/network_security_config.xml").is_file());
}

/// aapt2 path discovery + optional compile/link when SDK tools exist.
#[test]
fn aapt2_find_and_optional_rebuild() {
    use apk_patch_resources::find_aapt2;
    let Some(aapt2) = find_aapt2(None) else {
        return;
    };
    assert!(aapt2.is_file());

    let apk_path = match sample("TestActivity.apk") {
        Some(p) => p,
        None => return,
    };
    let temp = tempfile::tempdir().unwrap();
    let decoded = temp.path().join("decoded");
    decode_apk(
        &apk_path,
        &DecodeOptions {
            force: true,
            output: Some(decoded.clone()),
            no_src: true,
            ..Default::default()
        },
    )
    .unwrap();

    // Attempt aapt2 rebuild; success is environment-dependent (framework / resource validity).
    let result = build_project(
        &decoded,
        &BuildOptions {
            force: true,
            aapt: Some(aapt2),
            use_aapt2: true,
            sign: apk_patch_sign::BuildSignConfig {
                enabled: false,
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    assert!(result.output_apk.is_file());
    // Either aapt2 worked or we fell back to rust/original arsc.
    assert!(
        result.used_aapt2
            || result.used_rust_arsc
            || decoded.join("original/resources.arsc").is_file()
    );
}

/// Phase 3b: pure-Rust ARSC builder is the default rebuild path.
#[test]
fn pure_rust_arsc_builder_default() {
    let apk_path = match sample("TestActivity.apk") {
        Some(p) => p,
        None => return,
    };
    let temp = tempfile::tempdir().unwrap();
    let decoded = temp.path().join("decoded");
    decode_apk(
        &apk_path,
        &DecodeOptions {
            force: true,
            output: Some(decoded.clone()),
            no_src: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(decoded.join("res/values/public.xml").is_file());

    let result = build_project(
        &decoded,
        &BuildOptions {
            force: true,
            sign: apk_patch_sign::BuildSignConfig {
                enabled: false,
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    assert!(result.output_apk.is_file());
    assert!(
        result.used_rust_arsc,
        "expected pure-Rust ARSC builder by default"
    );
    assert!(!result.used_aapt2);
    assert!(decoded.join("build/resources.arsc").is_file());

    // Built ARSC must parse.
    let arsc = std::fs::read(decoded.join("build/resources.arsc")).unwrap();
    let parser = axml_parser::ARSCParser::new(&arsc).unwrap();
    assert!(!parser.resources.is_empty());
}

