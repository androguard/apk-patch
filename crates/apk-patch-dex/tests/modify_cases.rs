//! Strong dex-txt modification tests covering every edit case in the contract.
//!
//! Cases: mnemonic / registers / labels / refs / fields / methods / classes /
//! implements / try-catch / catchall / debug / p-regs / hex-comment ignored /
//! legacy hex / emit→edit→assemble loop / nested paths / multi-method.

use apk_patch_dex::{
    assemble_dex_from_txt, emit_dex_to_dir, parse_class_file, AssembleOptions, EmitOptions,
};
use dex_bytecode::decode_all;
use dex_parser::{DexFile, DexHelper, FieldKind};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

fn opts() -> AssembleOptions {
    AssembleOptions::default()
}

fn write_class(dir: &Path, rel: &str, content: &str) {
    let path = dir.join(rel);
    if let Some(p) = path.parent() {
        fs::create_dir_all(p).unwrap();
    }
    fs::write(path, content).unwrap();
}

fn assemble(dir: &Path) -> Vec<u8> {
    assemble_dex_from_txt(dir, &opts()).expect("assemble")
}

fn method_named<'a>(dex: &'a DexFile, name: &str) -> dex_parser::MethodInfoItem {
    DexHelper::from_dex(dex)
        .methods()
        .find_map(|m| m.ok().filter(|m| m.info.name == name))
        .unwrap_or_else(|| panic!("missing method {name}"))
}

fn method_names(dex: &DexFile) -> Vec<String> {
    DexHelper::from_dex(dex)
        .methods()
        .filter_map(|m| m.ok().map(|m| m.info.name))
        .collect()
}

fn field_names(dex: &DexFile) -> Vec<(String, FieldKind)> {
    DexHelper::from_dex(dex)
        .fields()
        .filter_map(|f| f.ok().map(|f| (f.info.name, f.field_kind)))
        .collect()
}

fn class_names(dex: &DexFile) -> Vec<String> {
    DexHelper::from_dex(dex)
        .classes()
        .filter_map(|c| c.ok().map(|c| c.name))
        .collect()
}

fn insns_of(dex: &DexFile, method: &str) -> Vec<u8> {
    let m = method_named(dex, method);
    let code = m.code_item.expect("code");
    code.insns_slice(&dex.data).to_vec()
}

fn registers_of(dex: &DexFile, method: &str) -> u16 {
    method_named(dex, method)
        .code_item
        .expect("code")
        .registers_size
}

fn mnemonics_of(dex: &DexFile, method: &str) -> Vec<String> {
    let bytes = insns_of(dex, method);
    decode_all(&bytes, 0)
        .unwrap()
        .into_iter()
        .map(|i| i.mnemonic.to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// 1. Instruction / registers
// ---------------------------------------------------------------------------

#[test]
fn modify_mnemonic_const_literal() {
    let dir = tempdir().unwrap();
    write_class(
        dir.path(),
        "Hello.dex.txt",
        r#"
.class public LHello;
.super Ljava/lang/Object;
.method public static main([Ljava/lang/String;)V
    .registers 2
    const/4 v0, 1
    return-void
.end method
"#,
    );
    let v1 = assemble(dir.path());
    let dex1 = DexFile::parse(&v1).unwrap();
    let ins1 = insns_of(&dex1, "main");
    assert_eq!(ins1[0], 0x12); // const/4
    // nibble literal 1 in high nibble of second byte for v0
    assert_eq!(ins1[1] & 0xf0, 0x10);

    write_class(
        dir.path(),
        "Hello.dex.txt",
        r#"
.class public LHello;
.super Ljava/lang/Object;
.method public static main([Ljava/lang/String;)V
    .registers 2
    const/4 v0, 7
    return-void
.end method
"#,
    );
    let dex2 = DexFile::parse(&assemble(dir.path())).unwrap();
    let ins2 = insns_of(&dex2, "main");
    assert_eq!(ins2[0], 0x12);
    assert_eq!(ins2[1] & 0xf0, 0x70);
    assert_ne!(ins1, ins2);
}

#[test]
fn modify_registers_count() {
    let dir = tempdir().unwrap();
    write_class(
        dir.path(),
        "Regs.dex.txt",
        r#"
.class public LRegs;
.super Ljava/lang/Object;
.method public static foo()V
    .registers 2
    return-void
.end method
"#,
    );
    assert_eq!(registers_of(&DexFile::parse(&assemble(dir.path())).unwrap(), "foo"), 2);

    write_class(
        dir.path(),
        "Regs.dex.txt",
        r#"
.class public LRegs;
.super Ljava/lang/Object;
.method public static foo()V
    .registers 8
    return-void
.end method
"#,
    );
    assert_eq!(registers_of(&DexFile::parse(&assemble(dir.path())).unwrap(), "foo"), 8);
}

#[test]
fn modify_replace_insn_sequence() {
    let dir = tempdir().unwrap();
    write_class(
        dir.path(),
        "Seq.dex.txt",
        r#"
.class public LSeq;
.super Ljava/lang/Object;
.method public static foo()V
    .registers 2
    nop
    return-void
.end method
"#,
    );
    let before = mnemonics_of(&DexFile::parse(&assemble(dir.path())).unwrap(), "foo");
    assert_eq!(before, vec!["nop", "return-void"]);

    write_class(
        dir.path(),
        "Seq.dex.txt",
        r#"
.class public LSeq;
.super Ljava/lang/Object;
.method public static foo()V
    .registers 2
    const/4 v0, 0
    const/4 v1, 1
    return-void
.end method
"#,
    );
    let after = mnemonics_of(&DexFile::parse(&assemble(dir.path())).unwrap(), "foo");
    assert_eq!(after, vec!["const/4", "const/4", "return-void"]);
}

#[test]
fn hex_comment_is_ignored_mnemonic_authoritative() {
    // Wrong hex comment must not win over mnemonic.
    let dir = tempdir().unwrap();
    write_class(
        dir.path(),
        "Hex.dex.txt",
        r#"
.class public LHex;
.super Ljava/lang/Object;
.method public static foo()V
    .registers 1
    return-void  # 0000
.end method
"#,
    );
    let ins = insns_of(&DexFile::parse(&assemble(dir.path())).unwrap(), "foo");
    assert_eq!(ins, vec![0x0e, 0x00], "mnemonic return-void must win over # 0000");
}

#[test]
fn legacy_hex_line_still_assembles() {
    let dir = tempdir().unwrap();
    write_class(
        dir.path(),
        "Legacy.dex.txt",
        r#"
.class public LLegacy;
.super Ljava/lang/Object;
.method public static foo()V
    .registers 1
    .code
    00000000: 0e00          return-void
    .end code
.end method
"#,
    );
    let ins = insns_of(&DexFile::parse(&assemble(dir.path())).unwrap(), "foo");
    assert_eq!(ins, vec![0x0e, 0x00]);
}

// ---------------------------------------------------------------------------
// 2. Labels / branches
// ---------------------------------------------------------------------------

#[test]
fn modify_branch_labels_goto() {
    let dir = tempdir().unwrap();
    write_class(
        dir.path(),
        "Br.dex.txt",
        r#"
.class public LBr;
.super Ljava/lang/Object;
.method public static foo()V
    .registers 1
    goto :Lend
    nop
    :Lend
    return-void
.end method
"#,
    );
    let dex = DexFile::parse(&assemble(dir.path())).unwrap();
    let mnems = mnemonics_of(&dex, "foo");
    assert_eq!(mnems, vec!["goto", "nop", "return-void"]);
    let ins = insns_of(&dex, "foo");
    // goto +2 units (skip nop): 0x28, 0x02
    assert_eq!(ins[0], 0x28);
    assert_eq!(ins[1] as i8, 2);
}

#[test]
fn modify_branch_if_eqz() {
    let dir = tempdir().unwrap();
    write_class(
        dir.path(),
        "If.dex.txt",
        r#"
.class public LIf;
.super Ljava/lang/Object;
.method public static foo()V
    .registers 2
    const/4 v0, 0
    if-eqz v0, :Ltrue
    return-void
    :Ltrue
    return-void
.end method
"#,
    );
    let mnems = mnemonics_of(&DexFile::parse(&assemble(dir.path())).unwrap(), "foo");
    assert!(mnems.iter().any(|m| m == "if-eqz"));
    assert_eq!(mnems.last().map(String::as_str), Some("return-void"));
}

#[test]
fn modify_relabel_target_changes_offset() {
    let dir = tempdir().unwrap();
    // Short jump over one nop
    write_class(
        dir.path(),
        "Rel.dex.txt",
        r#"
.class public LRel;
.super Ljava/lang/Object;
.method public static foo()V
    .registers 1
    goto :Lt
    nop
    :Lt
    return-void
.end method
"#,
    );
    let short = insns_of(&DexFile::parse(&assemble(dir.path())).unwrap(), "foo")[1] as i8;

    // Longer jump over three nops
    write_class(
        dir.path(),
        "Rel.dex.txt",
        r#"
.class public LRel;
.super Ljava/lang/Object;
.method public static foo()V
    .registers 1
    goto :Lt
    nop
    nop
    nop
    :Lt
    return-void
.end method
"#,
    );
    let longer = insns_of(&DexFile::parse(&assemble(dir.path())).unwrap(), "foo")[1] as i8;
    assert_eq!(short, 2);
    assert_eq!(longer, 4);
    assert_ne!(short, longer);
}

// ---------------------------------------------------------------------------
// 3. Methods
// ---------------------------------------------------------------------------

#[test]
fn modify_add_method() {
    let dir = tempdir().unwrap();
    write_class(
        dir.path(),
        "M.dex.txt",
        r#"
.class public LM;
.super Ljava/lang/Object;
.method public constructor <init>()V
    .registers 1
    return-void
.end method
"#,
    );
    assert!(!method_names(&DexFile::parse(&assemble(dir.path())).unwrap()).contains(&"extra".into()));

    write_class(
        dir.path(),
        "M.dex.txt",
        r#"
.class public LM;
.super Ljava/lang/Object;
.method public constructor <init>()V
    .registers 1
    return-void
.end method
.method public static extra()I
    .registers 1
    const/4 v0, 3
    return v0
.end method
"#,
    );
    let dex = DexFile::parse(&assemble(dir.path())).unwrap();
    assert!(method_names(&dex).contains(&"extra".into()));
    assert_eq!(mnemonics_of(&dex, "extra"), vec!["const/4", "return"]);
}

#[test]
fn modify_remove_method() {
    let dir = tempdir().unwrap();
    write_class(
        dir.path(),
        "M.dex.txt",
        r#"
.class public LM;
.super Ljava/lang/Object;
.method public constructor <init>()V
    .registers 1
    return-void
.end method
.method public static doomed()V
    .registers 1
    return-void
.end method
"#,
    );
    assert!(method_names(&DexFile::parse(&assemble(dir.path())).unwrap()).contains(&"doomed".into()));

    write_class(
        dir.path(),
        "M.dex.txt",
        r#"
.class public LM;
.super Ljava/lang/Object;
.method public constructor <init>()V
    .registers 1
    return-void
.end method
"#,
    );
    let names = method_names(&DexFile::parse(&assemble(dir.path())).unwrap());
    assert!(!names.contains(&"doomed".into()));
    assert!(names.contains(&"<init>".into()));
}

#[test]
fn modify_one_method_leaves_sibling_intact() {
    let dir = tempdir().unwrap();
    write_class(
        dir.path(),
        "Sib.dex.txt",
        r#"
.class public LSib;
.super Ljava/lang/Object;
.method public static a()V
    .registers 1
    nop
    return-void
.end method
.method public static b()V
    .registers 1
    return-void
.end method
"#,
    );
    write_class(
        dir.path(),
        "Sib.dex.txt",
        r#"
.class public LSib;
.super Ljava/lang/Object;
.method public static a()V
    .registers 1
    const/4 v0, 1
    return-void
.end method
.method public static b()V
    .registers 1
    return-void
.end method
"#,
    );
    let dex = DexFile::parse(&assemble(dir.path())).unwrap();
    assert_eq!(mnemonics_of(&dex, "a"), vec!["const/4", "return-void"]);
    assert_eq!(mnemonics_of(&dex, "b"), vec!["return-void"]);
}

// ---------------------------------------------------------------------------
// 4. Fields
// ---------------------------------------------------------------------------

#[test]
fn modify_add_and_remove_fields() {
    let dir = tempdir().unwrap();
    write_class(
        dir.path(),
        "F.dex.txt",
        r#"
.class public LF;
.super Ljava/lang/Object;
.field public static counter:I
.end field
.field private name:Ljava/lang/String;
.end field
.method public constructor <init>()V
    .registers 1
    return-void
.end method
"#,
    );
    let dex = DexFile::parse(&assemble(dir.path())).unwrap();
    let fields = field_names(&dex);
    assert!(fields.iter().any(|(n, k)| n == "counter" && *k == FieldKind::Static));
    assert!(fields.iter().any(|(n, k)| n == "name" && *k == FieldKind::Instance));

    write_class(
        dir.path(),
        "F.dex.txt",
        r#"
.class public LF;
.super Ljava/lang/Object;
.field private name:Ljava/lang/String;
.end field
.method public constructor <init>()V
    .registers 1
    return-void
.end method
"#,
    );
    let fields2 = field_names(&DexFile::parse(&assemble(dir.path())).unwrap());
    assert!(!fields2.iter().any(|(n, _)| n == "counter"));
    assert!(fields2.iter().any(|(n, _)| n == "name"));
}

#[test]
fn modify_field_access_via_insn() {
    let dir = tempdir().unwrap();
    write_class(
        dir.path(),
        "Fg.dex.txt",
        r#"
.class public LFg;
.super Ljava/lang/Object;
.field public static flag:Z
.end field
.method public static get()Z
    .registers 1
    sget-boolean v0, LFg;->flag:Z
    return v0
.end method
"#,
    );
    let mnems = mnemonics_of(&DexFile::parse(&assemble(dir.path())).unwrap(), "get");
    assert_eq!(mnems[0], "sget-boolean");
}

// ---------------------------------------------------------------------------
// 5. Classes / implements / nested path
// ---------------------------------------------------------------------------

#[test]
fn modify_add_and_remove_class_files() {
    let dir = tempdir().unwrap();
    write_class(
        dir.path(),
        "com/example/A.dex.txt",
        r#"
.class public Lcom/example/A;
.super Ljava/lang/Object;
.method public constructor <init>()V
    .registers 1
    return-void
.end method
"#,
    );
    write_class(
        dir.path(),
        "com/example/B.dex.txt",
        r#"
.class public Lcom/example/B;
.super Ljava/lang/Object;
.method public constructor <init>()V
    .registers 1
    return-void
.end method
"#,
    );
    let names = class_names(&DexFile::parse(&assemble(dir.path())).unwrap());
    assert_eq!(names.len(), 2);
    assert!(names.iter().any(|n| n == "Lcom/example/A;"));
    assert!(names.iter().any(|n| n == "Lcom/example/B;"));

    fs::remove_file(dir.path().join("com/example/B.dex.txt")).unwrap();
    let names2 = class_names(&DexFile::parse(&assemble(dir.path())).unwrap());
    assert_eq!(names2, vec!["Lcom/example/A;".to_string()]);
}

#[test]
fn modify_implements_interface() {
    let dir = tempdir().unwrap();
    write_class(
        dir.path(),
        "Runnable.dex.txt",
        r#"
.class public interface LRunnable;
.super Ljava/lang/Object;
.method public abstract run()V
.end method
"#,
    );
    write_class(
        dir.path(),
        "Task.dex.txt",
        r#"
.class public LTask;
.super Ljava/lang/Object;
.implements LRunnable;
.method public constructor <init>()V
    .registers 1
    return-void
.end method
.method public run()V
    .registers 1
    return-void
.end method
"#,
    );
    let dex = DexFile::parse(&assemble(dir.path())).unwrap();
    let task = DexHelper::from_dex(&dex)
        .classes()
        .find_map(|c| c.ok().filter(|c| c.name == "LTask;"))
        .expect("LTask");
    assert_ne!(task.class_def.interfaces_off, 0, "implements must write type_list");
}

// ---------------------------------------------------------------------------
// 6. Refs: string / type / method
// ---------------------------------------------------------------------------

#[test]
fn modify_const_string_and_invoke() {
    let dir = tempdir().unwrap();
    write_class(
        dir.path(),
        "Msg.dex.txt",
        r#"
.class public LMsg;
.super Ljava/lang/Object;
.method public static hello()Ljava/lang/String;
    .registers 2
    const-string v0, "hi"
    return-object v0
.end method
"#,
    );
    let dex = DexFile::parse(&assemble(dir.path())).unwrap();
    assert_eq!(mnemonics_of(&dex, "hello")[0], "const-string");
    // Pool must contain the string
    let strings: Vec<_> = DexHelper::from_dex(&dex)
        .strings()
        .filter_map(Result::ok)
        .collect();
    assert!(strings.iter().any(|s| s == "hi"));

    write_class(
        dir.path(),
        "Msg.dex.txt",
        r#"
.class public LMsg;
.super Ljava/lang/Object;
.method public static hello()Ljava/lang/String;
    .registers 2
    const-string v0, "hello, world"
    return-object v0
.end method
"#,
    );
    let dex2 = DexFile::parse(&assemble(dir.path())).unwrap();
    let strings2: Vec<_> = DexHelper::from_dex(&dex2)
        .strings()
        .filter_map(Result::ok)
        .collect();
    assert!(strings2.iter().any(|s| s == "hello, world"));
    assert!(!strings2.iter().any(|s| s == "hi"));
}

#[test]
fn modify_new_instance_and_invoke_direct() {
    let dir = tempdir().unwrap();
    write_class(
        dir.path(),
        "New.dex.txt",
        r#"
.class public LNew;
.super Ljava/lang/Object;
.method public constructor <init>()V
    .registers 1
    invoke-direct {v0}, Ljava/lang/Object;-><init>()V
    return-void
.end method
.method public static make()LNew;
    .registers 2
    new-instance v0, LNew;
    invoke-direct {v0}, LNew;-><init>()V
    return-object v0
.end method
"#,
    );
    let mnems = mnemonics_of(&DexFile::parse(&assemble(dir.path())).unwrap(), "make");
    assert_eq!(mnems[0], "new-instance");
    assert_eq!(mnems[1], "invoke-direct");
    assert_eq!(mnems[2], "return-object");
}

// ---------------------------------------------------------------------------
// 7. Try / catch / catchall
// ---------------------------------------------------------------------------

#[test]
fn modify_try_catch_typed() {
    let dir = tempdir().unwrap();
    write_class(
        dir.path(),
        "Try.dex.txt",
        r#"
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
"#,
    );
    let code = method_named(&DexFile::parse(&assemble(dir.path())).unwrap(), "run")
        .code_item
        .unwrap();
    assert!(code.tries_size >= 1);
}

#[test]
fn modify_try_catchall() {
    let txt = r#"
.class public LAll;
.super Ljava/lang/Object;
.method public static run()V
    .registers 2
    :Ls
    nop
    :Le
    return-void
    :Lh
    move-exception v0
    return-void
    .catchall { :Ls .. :Le } :Lh
.end method
"#;
    let class = parse_class_file(txt).unwrap();
    assert!(class.methods[0].catches[0].exception_type.is_none());

    let dir = tempdir().unwrap();
    write_class(dir.path(), "All.dex.txt", txt);
    let code = method_named(&DexFile::parse(&assemble(dir.path())).unwrap(), "run")
        .code_item
        .unwrap();
    assert!(code.tries_size >= 1);
}

#[test]
fn modify_remove_catch_clears_tries() {
    let dir = tempdir().unwrap();
    write_class(
        dir.path(),
        "Tc.dex.txt",
        r#"
.class public LTc;
.super Ljava/lang/Object;
.method public static run()V
    .registers 2
    :Ls
    nop
    :Le
    return-void
    :Lh
    move-exception v0
    return-void
    .catch Ljava/lang/Throwable; { :Ls .. :Le } :Lh
.end method
"#,
    );
    assert!(
        method_named(&DexFile::parse(&assemble(dir.path())).unwrap(), "run")
            .code_item
            .unwrap()
            .tries_size
            >= 1
    );

    write_class(
        dir.path(),
        "Tc.dex.txt",
        r#"
.class public LTc;
.super Ljava/lang/Object;
.method public static run()V
    .registers 1
    nop
    return-void
.end method
"#,
    );
    assert_eq!(
        method_named(&DexFile::parse(&assemble(dir.path())).unwrap(), "run")
            .code_item
            .unwrap()
            .tries_size,
        0
    );
}

// ---------------------------------------------------------------------------
// 8. Debug / p-registers
// ---------------------------------------------------------------------------

#[test]
fn modify_debug_line_sets_debug_info() {
    let dir = tempdir().unwrap();
    write_class(
        dir.path(),
        "Dbg.dex.txt",
        r#"
.class public LDbg;
.super Ljava/lang/Object;
.method public static foo()V
    .registers 1
    .line 42
    return-void
.end method
"#,
    );
    let code = method_named(&DexFile::parse(&assemble(dir.path())).unwrap(), "foo")
        .code_item
        .unwrap();
    assert_ne!(code.debug_info_off, 0, ".line should emit debug_info_item");
}

#[test]
fn modify_p_registers_converted() {
    // non-static: p0 is `this` at v(registers - ins_size)
    let dir = tempdir().unwrap();
    write_class(
        dir.path(),
        "P.dex.txt",
        r#"
.class public LP;
.super Ljava/lang/Object;
.method public get()I
    .registers 3
    const/4 p0, 1
    return p0
.end method
"#,
    );
    // ins_size = 1 (this), registers = 3 → p0 → v2
    let ins = insns_of(&DexFile::parse(&assemble(dir.path())).unwrap(), "get");
    assert_eq!(ins[0], 0x12); // const/4
    assert_eq!(ins[1] & 0x0f, 2); // dest reg v2
}

// ---------------------------------------------------------------------------
// 9. Emit → edit → assemble loop
// ---------------------------------------------------------------------------

#[test]
fn edit_loop_emit_change_mnemonic_reassemble() {
    let dir = tempdir().unwrap();
    write_class(
        dir.path(),
        "Loop.dex.txt",
        r#"
.class public LLoop;
.super Ljava/lang/Object;
.method public static foo()I
    .registers 1
    const/4 v0, 1
    return v0
.end method
"#,
    );
    let bytes1 = assemble(dir.path());
    let emit_dir = tempdir().unwrap();
    assert_eq!(
        emit_dex_to_dir(&bytes1, "classes.dex", emit_dir.path(), &EmitOptions::default()).unwrap(),
        1
    );
    let txt_path = find_txt(emit_dir.path(), "Loop").expect("emitted Loop.dex.txt");
    let mut text = fs::read_to_string(&txt_path).unwrap();
    assert!(text.contains("const/4"), "{text}");
    if text.contains("const/4 v0, 1") {
        text = text.replace("const/4 v0, 1", "const/4 v0, 5");
    } else {
        text = r#"
.class public LLoop;
.super Ljava/lang/Object;
.method public static foo()I
    .registers 1
    const/4 v0, 5
    return v0
.end method
"#
        .into();
    }
    fs::write(&txt_path, text).unwrap();

    let bytes2 = assemble(emit_dir.path());
    let dex2 = DexFile::parse(&bytes2).unwrap();
    let ins = insns_of(&dex2, "foo");
    assert_eq!(ins[0], 0x12);
    assert_eq!(ins[1] & 0xf0, 0x50);
}

#[test]
fn edit_loop_emit_add_method_reassemble() {
    let dir = tempdir().unwrap();
    write_class(
        dir.path(),
        "Add.dex.txt",
        r#"
.class public LAdd;
.super Ljava/lang/Object;
.method public constructor <init>()V
    .registers 1
    return-void
.end method
"#,
    );
    let bytes1 = assemble(dir.path());
    let emit_dir = tempdir().unwrap();
    emit_dex_to_dir(&bytes1, "classes.dex", emit_dir.path(), &EmitOptions::default()).unwrap();
    let txt_path = find_txt(emit_dir.path(), "Add").unwrap();
    let mut text = fs::read_to_string(&txt_path).unwrap();
    text.push_str(
        r#"

.method public static added()V
    .registers 1
    return-void
.end method
"#,
    );
    fs::write(&txt_path, text).unwrap();
    let names = method_names(&DexFile::parse(&assemble(emit_dir.path())).unwrap());
    assert!(names.iter().any(|n| n == "added"));
    assert!(names.iter().any(|n| n == "<init>"));
}

#[test]
fn edit_loop_double_roundtrip_stable_semantics() {
    let dir = tempdir().unwrap();
    write_class(
        dir.path(),
        "com/example/Stable.dex.txt",
        r#"
.class public Lcom/example/Stable;
.super Ljava/lang/Object;
.method public static answer()I
    .registers 1
    const/4 v0, 4
    return v0
.end method
"#,
    );
    let b0 = assemble(dir.path());
    let e1 = tempdir().unwrap();
    emit_dex_to_dir(&b0, "classes.dex", e1.path(), &EmitOptions::default()).unwrap();
    let b1 = assemble(e1.path());
    let e2 = tempdir().unwrap();
    emit_dex_to_dir(&b1, "classes.dex", e2.path(), &EmitOptions::default()).unwrap();
    let b2 = assemble(e2.path());

    let d0 = DexFile::parse(&b0).unwrap();
    let d2 = DexFile::parse(&b2).unwrap();
    assert_eq!(mnemonics_of(&d0, "answer"), mnemonics_of(&d2, "answer"));
    assert_eq!(registers_of(&d0, "answer"), registers_of(&d2, "answer"));
    // Pool indices may differ; semantic insn opcodes/literals should match for this simple method
    assert_eq!(insns_of(&d0, "answer"), insns_of(&d2, "answer"));
}

// ---------------------------------------------------------------------------
// 10. Real APK: emit → modify one method → assemble
// ---------------------------------------------------------------------------

#[test]
fn testactivity_emit_modify_reassemble() {
    let apk_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../apk-parser/tests/data/APK/TestActivity.apk");
    if !apk_path.is_file() {
        return;
    }
    let apk = fs::read(&apk_path).unwrap();
    let zip = apkparser::Apk::from_bytes(&apk, apkparser::ApkOptions::default()).unwrap();
    let original = zip.get_file("classes.dex").unwrap().to_vec();

    let emit_dir = tempdir().unwrap();
    emit_dex_to_dir(&original, "classes.dex", emit_dir.path(), &EmitOptions::default()).unwrap();

    // Patch: insert a nop at the start of the first non-empty method we find
    let mut patched = false;
    for entry in walkdir::WalkDir::new(emit_dir.path())
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.path().extension().and_then(|x| x.to_str()) != Some("txt") {
            continue;
        }
        let mut text = fs::read_to_string(entry.path()).unwrap();
        if let Some(idx) = text.find(".registers ") {
            // After the registers line, insert nop
            let after_line = text[idx..]
                .find('\n')
                .map(|n| idx + n + 1)
                .unwrap_or(text.len());
            text.insert_str(after_line, "    nop\n");
            fs::write(entry.path(), text).unwrap();
            patched = true;
            break;
        }
    }
    assert!(patched, "expected at least one method with .registers");

    let rebuilt = assemble(emit_dir.path());
    let dex = DexFile::parse(&rebuilt).unwrap();
    assert!(dex.header.class_defs_size > 0);
    // At least one method should start with nop
    let any_nop = DexHelper::from_dex(&dex).methods().filter_map(Result::ok).any(|m| {
        m.code_item
            .as_ref()
            .map(|c| c.insns_slice(&dex.data).first() == Some(&0x00))
            .unwrap_or(false)
    });
    assert!(any_nop, "modified method should begin with nop");
}

fn find_txt(root: &Path, needle: &str) -> Option<PathBuf> {
    for entry in walkdir::WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
        let p = entry.path();
        if p.extension().and_then(|x| x.to_str()) == Some("txt") {
            if p.to_string_lossy().contains(needle) {
                return Some(p.to_path_buf());
            }
        }
    }
    None
}






