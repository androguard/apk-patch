//! Structural assemble-from-scratch tests for dex-txt.

use apk_patch_dex::{assemble_dex_from_txt, emit_dex_to_dir, parse_class_file, AssembleOptions, EmitOptions};
use dex_parser::{DexFile, DexHelper};
use std::fs;
use tempfile::tempdir;

fn write_class(dir: &std::path::Path, rel: &str, content: &str) {
    let path = dir.join(rel);
    if let Some(p) = path.parent() {
        fs::create_dir_all(p).unwrap();
    }
    fs::write(path, content).unwrap();
}

#[test]
fn mnemonic_edit_and_registers() {
    let dir = tempdir().unwrap();
    write_class(
        dir.path(),
        "LHello;.dex.txt",
        r#"
.class public LHello;
.super Ljava/lang/Object;

.method public static main([Ljava/lang/String;)V
    .registers 3
    const/4 v0, 1
    return-void
.end method
"#,
    );
    let bytes = assemble_dex_from_txt(dir.path(), &AssembleOptions::default()).unwrap();
    let dex = DexFile::parse(&bytes).unwrap();
    let helper = DexHelper::from_dex(&dex);
    let method = helper
        .methods()
        .find_map(|m| m.ok().filter(|m| m.info.name == "main"))
        .expect("main");
    let code = method.code_item.expect("code");
    assert_eq!(code.registers_size, 3);
    assert_eq!(code.insns_slice(&dex.data)[0], 0x12); // const/4
}

#[test]
fn add_and_remove_method() {
    let dir = tempdir().unwrap();
    write_class(
        dir.path(),
        "LFoo;.dex.txt",
        r#"
.class public LFoo;
.super Ljava/lang/Object;

.method public constructor <init>()V
    .registers 1
    return-void
.end method

.method public static added()V
    .registers 1
    return-void
.end method
"#,
    );
    let bytes = assemble_dex_from_txt(dir.path(), &AssembleOptions::default()).unwrap();
    let dex = DexFile::parse(&bytes).unwrap();
    let names: Vec<_> = DexHelper::from_dex(&dex)
        .methods()
        .filter_map(|m| m.ok().map(|m| m.info.name))
        .collect();
    assert!(names.iter().any(|n| n == "added"));
    assert!(names.iter().any(|n| n == "<init>"));

    // Remove added method
    write_class(
        dir.path(),
        "LFoo;.dex.txt",
        r#"
.class public LFoo;
.super Ljava/lang/Object;

.method public constructor <init>()V
    .registers 1
    return-void
.end method
"#,
    );
    let bytes2 = assemble_dex_from_txt(dir.path(), &AssembleOptions::default()).unwrap();
    let dex2 = DexFile::parse(&bytes2).unwrap();
    let names2: Vec<_> = DexHelper::from_dex(&dex2)
        .methods()
        .filter_map(|m| m.ok().map(|m| m.info.name))
        .collect();
    assert!(!names2.iter().any(|n| n == "added"));
}

#[test]
fn add_and_remove_class() {
    let dir = tempdir().unwrap();
    write_class(
        dir.path(),
        "LA;.dex.txt",
        r#"
.class public LA;
.super Ljava/lang/Object;
.method public constructor <init>()V
    .registers 1
    return-void
.end method
"#,
    );
    write_class(
        dir.path(),
        "LB;.dex.txt",
        r#"
.class public LB;
.super Ljava/lang/Object;
.method public constructor <init>()V
    .registers 1
    return-void
.end method
"#,
    );
    let bytes = assemble_dex_from_txt(dir.path(), &AssembleOptions::default()).unwrap();
    assert_eq!(DexFile::parse(&bytes).unwrap().header.class_defs_size, 2);

    fs::remove_file(dir.path().join("LB;.dex.txt")).unwrap();
    let bytes2 = assemble_dex_from_txt(dir.path(), &AssembleOptions::default()).unwrap();
    let dex2 = DexFile::parse(&bytes2).unwrap();
    assert_eq!(dex2.header.class_defs_size, 1);
    assert_eq!(dex2.get_type(dex2.get_class_def(0).unwrap().class_idx).unwrap(), "LA;");
}

#[test]
fn try_catch_roundtrip_txt() {
    let txt = r#"
.class public LTry;
.super Ljava/lang/Object;

.method public static run()V
    .registers 2
    :Lstart
    const/4 v0, 0
    :Lend
    return-void
    :Lhandler
    move-exception v0
    return-void
    .catch Ljava/lang/Exception; { :Lstart .. :Lend } :Lhandler
.end method
"#;
    let class = parse_class_file(txt).unwrap();
    assert_eq!(class.methods[0].catches.len(), 1);
    let dir = tempdir().unwrap();
    write_class(dir.path(), "LTry;.dex.txt", txt);
    let bytes = assemble_dex_from_txt(dir.path(), &AssembleOptions::default()).unwrap();
    let dex = DexFile::parse(&bytes).unwrap();
    let method = DexHelper::from_dex(&dex)
        .methods()
        .find_map(|m| m.ok().filter(|m| m.info.name == "run"))
        .unwrap();
    let code = method.code_item.unwrap();
    assert!(code.tries_size >= 1);
}

#[test]
fn emit_assemble_roundtrip_minimal() {
    let dir = tempdir().unwrap();
    write_class(
        dir.path(),
        "LRound;.dex.txt",
        r#"
.class public LRound;
.super Ljava/lang/Object;

.method public static foo()I
    .registers 1
    const/4 v0, 2
    return v0
.end method
"#,
    );
    let bytes = assemble_dex_from_txt(dir.path(), &AssembleOptions::default()).unwrap();
    let out = tempdir().unwrap();
    emit_dex_to_dir(&bytes, "classes.dex", out.path(), &EmitOptions::default()).unwrap();
    let bytes2 = assemble_dex_from_txt(out.path(), &AssembleOptions::default()).unwrap();
    let dex = DexFile::parse(&bytes2).unwrap();
    assert_eq!(dex.header.class_defs_size, 1);
    let method = DexHelper::from_dex(&dex)
        .methods()
        .find_map(|m| m.ok().filter(|m| m.info.name == "foo"))
        .unwrap();
    let code = method.code_item.unwrap();
    assert_eq!(code.insns_slice(&dex.data)[0], 0x12);
}

#[test]
fn emit_assemble_testactivity_smoke() {
    let apk_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../apk-parser/tests/data/APK/TestActivity.apk");
    if !apk_path.is_file() {
        return;
    }
    let apk = fs::read(&apk_path).unwrap();
    let zip = apkparser::Apk::from_bytes(&apk, apkparser::ApkOptions::default()).unwrap();
    let original = zip.get_file("classes.dex").unwrap().to_vec();
    let out = tempdir().unwrap();
    let n = emit_dex_to_dir(&original, "classes.dex", out.path(), &EmitOptions::default()).unwrap();
    assert!(n > 0);
    let rebuilt = assemble_dex_from_txt(out.path(), &AssembleOptions::default());
    match rebuilt {
        Ok(bytes) => {
            let dex = DexFile::parse(&bytes).expect("rebuilt dex parses");
            assert!(dex.header.class_defs_size > 0);
        }
        Err(e) => {
            // Full opcode coverage may still miss rare formats; surface clearly.
            eprintln!("TestActivity assemble not yet complete: {e}");
            // Prefer soft-fail only if error is encode-related for exotic opcodes
            panic!("TestActivity emit→assemble failed: {e}");
        }
    }
}

