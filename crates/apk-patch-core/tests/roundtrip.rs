//! Integration tests for Phase 0/1 decode/build roundtrip.

use std::path::Path;

use apk_patch_core::{build_project, decode_apk, BuildOptions, DecodeOptions};
use apkparser::{Apk, ApkOptions};
use dex_parser::{DexFile, DexHelper};

fn test_apk_path() -> Option<std::path::PathBuf> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../apk-parser/tests/data/APK/TestActivity.apk");
    if path.is_file() {
        Some(path)
    } else {
        None
    }
}

#[test]
fn decode_build_roundtrip_signed() {
    let apk_path = match test_apk_path() {
        Some(p) => p,
        None => return,
    };

    let temp = tempfile::tempdir().unwrap();
    let out = temp.path().join("decoded");

    let decode_result = decode_apk(
        &apk_path,
        &DecodeOptions {
            force: true,
            output: Some(out.clone()),
            no_res: true,
            ..Default::default()
        },
    )
    .unwrap();

    assert!(decode_result.output_dir.join("apkpatch.yml").is_file());
    assert!(decode_result.output_dir.join("AndroidManifest.xml").is_file());
    let manifest = std::fs::read_to_string(decode_result.output_dir.join("AndroidManifest.xml")).unwrap();
    assert!(manifest.starts_with("<?xml") || manifest.trim_start().starts_with("<manifest"));
    assert!(decode_result.output_dir.join("original/AndroidManifest.xml").is_file());
    assert!(decode_result.output_dir.join("dex").is_dir());
    assert!(decode_result.output_dir.join("original/classes.dex").is_file());
    assert!(decode_result.dex_class_count > 0);

    let build_result = build_project(
        &decode_result.output_dir,
        &BuildOptions {
            force: true,
            ..Default::default()
        },
    )
    .unwrap();

    assert!(build_result.signed);
    assert!(build_result.output_apk.is_file());

    let rebuilt = std::fs::read(&build_result.output_apk).unwrap();
    let apk = Apk::from_bytes(
        &rebuilt,
        ApkOptions::default().with_signature(true),
    )
    .unwrap();
    let sig = apk.get_signature().expect("signature parsed");
    assert!(sig.is_signed_v1() || sig.is_signed_v2() || sig.is_signed_v3());
}

#[test]
fn dex_txt_roundtrip_preserves_classes() {
    let apk_path = match test_apk_path() {
        Some(p) => p,
        None => return,
    };

    let original_apk = std::fs::read(&apk_path).unwrap();
    let original_dex = Apk::from_bytes(&original_apk, ApkOptions::default())
        .unwrap()
        .get_file("classes.dex")
        .unwrap();
    let orig_parsed = DexFile::parse(&original_dex).unwrap();
    let orig_class_count = orig_parsed.header.class_defs_size;

    let temp = tempfile::tempdir().unwrap();
    let out = temp.path().join("decoded");
    decode_apk(
        &apk_path,
        &DecodeOptions {
            force: true,
            output: Some(out.clone()),
            no_res: true,
            ..Default::default()
        },
    )
    .unwrap();

    let build_result = build_project(
        &out,
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

    let rebuilt_apk = std::fs::read(&build_result.output_apk).unwrap();
    let rebuilt_dex = Apk::from_bytes(&rebuilt_apk, ApkOptions::default())
        .unwrap()
        .get_file("classes.dex")
        .unwrap();
    let rebuilt_parsed = DexFile::parse(&rebuilt_dex).unwrap();
    assert_eq!(rebuilt_parsed.header.class_defs_size, orig_class_count);
}

#[test]
fn dex_txt_edit_nop_patch() {
    let apk_path = match test_apk_path() {
        Some(p) => p,
        None => return,
    };

    let temp = tempfile::tempdir().unwrap();
    let out = temp.path().join("decoded");
    decode_apk(
        &apk_path,
        &DecodeOptions {
            force: true,
            output: Some(out.clone()),
            no_res: true,
            ..Default::default()
        },
    )
    .unwrap();

    // Find any dex-txt with return-void and patch to nop (same 2-byte size).
    let mut patched = false;
    for entry in walkdir::WalkDir::new(out.join("dex")).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let mut content = std::fs::read_to_string(path).unwrap();
        if content.contains("0e00") && content.contains("return-void") {
            content = content.replace("0e00 return-void", "0000 nop");
            std::fs::write(path, content).unwrap();
            patched = true;
            break;
        }
    }
    assert!(patched, "expected at least one return-void to patch");

    let build_result = build_project(
        &out,
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

    let rebuilt_dex = Apk::from_bytes(
        &std::fs::read(&build_result.output_apk).unwrap(),
        ApkOptions::default(),
    )
    .unwrap()
    .get_file("classes.dex")
    .unwrap();

    let dex = DexFile::parse(&rebuilt_dex).unwrap();
    let helper = DexHelper::from_dex(&dex);
    let has_nop = helper.methods().any(|m| {
        let Ok(m) = m else { return false };
        let Some(code) = m.code_item.as_ref() else { return false };
        code.insns_slice(&dex.data).windows(2).any(|w| w == [0x00, 0x00])
    });
    assert!(has_nop);
}

#[test]
fn dex_txt_variable_size_insn_edit() {
    let apk_path = match test_apk_path() {
        Some(p) => p,
        None => return,
    };

    let temp = tempfile::tempdir().unwrap();
    let out = temp.path().join("decoded");
    decode_apk(
        &apk_path,
        &DecodeOptions {
            force: true,
            output: Some(out.clone()),
            no_res: true,
            ..Default::default()
        },
    )
    .unwrap();

    let target = out.join("dex/tests/androguard/TestActivity.dex.txt");
    let mut content = std::fs::read_to_string(&target).unwrap();
    content = content.replace(
        ".method public bridge testVarArgs(I[J[Ljava/lang/String;)V\n    .registers 4\n    .code\n    00000000: 0e00          return-void",
        ".method public bridge testVarArgs(I[J[Ljava/lang/String;)V\n    .registers 4\n    .code\n    00000000: 13000000      const/16 v0, 0",
    );
    assert!(
        !content.contains("00000000: 0e00          return-void"),
        "expected testVarArgs patch to apply"
    );
    std::fs::write(&target, content).unwrap();

    let build_result = build_project(
        &out,
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

    let rebuilt_dex = Apk::from_bytes(
        &std::fs::read(&build_result.output_apk).unwrap(),
        ApkOptions::default(),
    )
    .unwrap()
    .get_file("classes.dex")
    .unwrap();

    let dex = DexFile::parse(&rebuilt_dex).unwrap();
    let helper = DexHelper::from_dex(&dex);
    let varargs = helper.methods().find_map(|m| {
        let m = m.ok()?;
        if m.info.class.contains("TestActivity") && m.info.name == "testVarArgs" {
            Some(m)
        } else {
            None
        }
    });
    let varargs = varargs.expect("testVarArgs method");
    let code = varargs.code_item.expect("code item");
    assert_eq!(code.insns_size, 2, "expected 4-byte const/16 (2 code units)");
    assert_eq!(
        code.insns_slice(&dex.data),
        &[0x13, 0x00, 0x00, 0x00]
    );
}

#[test]
fn no_src_copies_raw_dex() {
    let apk_path = match test_apk_path() {
        Some(p) => p,
        None => return,
    };

    let temp = tempfile::tempdir().unwrap();
    let out = temp.path().join("decoded");
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

    assert!(out.join("classes.dex").is_file());
    assert!(!out.join("dex").exists());
}

#[test]
fn only_manifest_decode() {
    let apk_path = match test_apk_path() {
        Some(p) => p,
        None => return,
    };

    let temp = tempfile::tempdir().unwrap();
    let out = temp.path().join("decoded");
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
    assert!(out.join("apkpatch.yml").is_file());
    assert!(!out.join("dex").exists());
    assert!(!out.join("classes.dex").exists());

    let manifest = std::fs::read_to_string(out.join("AndroidManifest.xml")).unwrap();
    assert!(manifest.contains("manifest"));
}
