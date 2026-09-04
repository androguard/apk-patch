//! Integration tests for dex-txt gap closure (annotations, static values, payloads, verify).

use apk_patch_dex::{
    assemble_dex_from_txt, parse_class_file, verify_method, AssembleOptions,
};
use std::fs;
use tempfile::tempdir;

fn assemble_one(txt: &str) -> Vec<u8> {
    let dir = tempdir().unwrap();
    let path = dir.path().join("Hello.dex.txt");
    fs::write(&path, txt).unwrap();
    assemble_dex_from_txt(dir.path(), &AssembleOptions::default()).expect("assemble")
}

#[test]
fn static_value_and_annotation_roundtrip() {
    let txt = r#"
.class public LHello;
.super Ljava/lang/Object;

.annotation runtime Ljava/lang/Deprecated;
.end annotation

.field public static final MSG:Ljava/lang/String;
    .value "hi"
.end field

.method public constructor <init>()V
    .registers 1
    return-void
.end method
"#;
    let bytes = assemble_one(txt);
    let dex = dex_parser::DexFile::parse(&bytes).unwrap();
    assert_eq!(dex.header.class_defs_size, 1);
    let cd = dex.get_class_def(0).unwrap();
    assert_ne!(cd.annotations_off, 0);
    assert_ne!(cd.static_values_off, 0);
    let anns = dex.get_annotations(&cd).unwrap();
    assert!(!anns.class_annotations.is_empty());
    let vals = dex.get_static_values(&cd).unwrap();
    assert_eq!(vals.len(), 1);
}

#[test]
fn array_data_payload_assembles() {
    let txt = r#"
.class public LHello;
.super Ljava/lang/Object;

.method public static fill()V
    .registers 2
    const/4 v1, 2
    new-array v0, v1, [I
    fill-array-data v0, :Ldata
    return-void
    :Ldata
    .array-data 4
        0x1
        0x2
    .end array-data
.end method
"#;
    let bytes = assemble_one(txt);
    let dex = dex_parser::DexFile::parse(&bytes).unwrap();
    assert_eq!(dex.header.class_defs_size, 1);
    let cd = dex.get_class_def(0).unwrap();
    let class_data = dex.get_class_data(&cd).unwrap().unwrap();
    let m = &class_data.direct_methods[0];
    let code = dex.get_code_item(m.code_off).unwrap();
    let insns = code.insns_slice(&dex.data);
    assert!(
        insns.windows(2).any(|w| w == [0x00, 0x03]),
        "expected fill-array-data payload ident, got {insns:02x?}"
    );
}

#[test]
fn verify_rejects_bad_label() {
    let txt = r#"
.class public LHello;
.super Ljava/lang/Object;

.method public foo()V
    .registers 1
    goto :Lmissing
    return-void
.end method
"#;
    let class = parse_class_file(txt).unwrap();
    assert!(verify_method(&class.methods[0], &class.class_descriptor).is_err());
}

#[test]
fn locals_directive() {
    let txt = r#"
.class public LHello;
.super Ljava/lang/Object;

.method public static bar(I)V
    .locals 1
    return-void
.end method
"#;
    let class = parse_class_file(txt).unwrap();
    assert_eq!(class.methods[0].registers, Some(2));
}
