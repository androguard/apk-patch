//! Strong tests for baksmali-style register-type dataflow verification.

use apk_patch_dex::{parse_class_file, verify_method, Category, RegType};

fn check(txt: &str) -> Result<(), String> {
    let class = parse_class_file(txt).map_err(|e| e.to_string())?;
    verify_method(&class.methods[0], &class.class_descriptor).map_err(|e| e.to_string())
}

fn assert_ok(txt: &str) {
    check(txt).unwrap_or_else(|e| panic!("expected OK, got: {e}\n{txt}"));
}

fn assert_err_contains(txt: &str, needle: &str) {
    let err = check(txt).expect_err("expected verification failure");
    assert!(
        err.contains(needle),
        "error `{err}` does not contain `{needle}`\n{txt}"
    );
}

#[test]
fn lattice_merge_cases() {
    assert_eq!(
        RegType::cat(Category::Byte)
            .merge(&RegType::cat(Category::Char))
            .category,
        Category::Integer
    );
    assert_eq!(
        RegType::cat(Category::Integer)
            .merge(&RegType::reference("Ljava/lang/Object;"))
            .category,
        Category::Conflicted
    );
    assert_eq!(
        RegType::cat(Category::Null)
            .merge(&RegType::reference("Ljava/lang/String;"))
            .type_desc
            .as_deref(),
        Some("Ljava/lang/String;")
    );
    assert_eq!(
        RegType::cat(Category::LongLo)
            .merge(&RegType::cat(Category::DoubleLo))
            .category,
        Category::LongLo
    );
}

#[test]
fn ok_simple_const_return() {
    assert_ok(
        r#"
.class public LT;
.super Ljava/lang/Object;
.method public static foo()I
    .registers 1
    const/4 v0, 1
    return v0
.end method
"#,
    );
}

#[test]
fn ok_wide_long_roundtrip() {
    assert_ok(
        r#"
.class public LT;
.super Ljava/lang/Object;
.method public static foo()J
    .registers 2
    const-wide/16 v0, 1
    return-wide v0
.end method
"#,
    );
}

#[test]
fn err_wide_hi_used_alone() {
    assert_err_contains(
        r#"
.class public LT;
.super Ljava/lang/Object;
.method public static foo()I
    .registers 2
    const-wide/16 v0, 1
    return v1
.end method
"#,
        "register-type",
    );
}

#[test]
fn err_overwrite_wide_lo_breaks_pair() {
    assert_err_contains(
        r#"
.class public LT;
.super Ljava/lang/Object;
.method public static foo()J
    .registers 2
    const-wide/16 v0, 1
    const/4 v0, 0
    return-wide v0
.end method
"#,
        "wide",
    );
}

#[test]
fn err_move_result_without_invoke() {
    assert_err_contains(
        r#"
.class public LT;
.super Ljava/lang/Object;
.method public static foo()I
    .registers 1
    move-result v0
    return v0
.end method
"#,
        "move-result without pending",
    );
}

#[test]
fn ok_invoke_static_move_result() {
    assert_ok(
        r#"
.class public LT;
.super Ljava/lang/Object;
.method public static foo()I
    .registers 1
    invoke-static {}, LT;->bar()I
    move-result v0
    return v0
.end method
.method public static bar()I
    .registers 1
    const/4 v0, 2
    return v0
.end method
"#,
    );
}

#[test]
fn err_return_void_wrong() {
    assert_err_contains(
        r#"
.class public LT;
.super Ljava/lang/Object;
.method public static foo()I
    .registers 1
    return-void
.end method
"#,
        "return-void",
    );
}

#[test]
fn err_return_wide_for_int_method() {
    assert_err_contains(
        r#"
.class public LT;
.super Ljava/lang/Object;
.method public static foo()I
    .registers 2
    const-wide/16 v0, 0
    return-wide v0
.end method
"#,
        "return-wide",
    );
}

#[test]
fn err_use_uninit_local() {
    assert_err_contains(
        r#"
.class public LT;
.super Ljava/lang/Object;
.method public static foo()I
    .registers 2
    return v0
.end method
"#,
        "register-type",
    );
}

#[test]
fn err_conflict_merge_at_join() {
    // Branch: one path puts int in v0, other puts Object; join then return v0 as int.
    assert_err_contains(
        r#"
.class public LT;
.super Ljava/lang/Object;
.method public static foo(Z)I
    .registers 2
    if-eqz v1, :Lobj
    const/4 v0, 1
    goto :Ljoin
    :Lobj
    const-string v0, "x"
    :Ljoin
    return v0
.end method
"#,
        "register-type",
    );
}

#[test]
fn ok_branch_both_int() {
    assert_ok(
        r#"
.class public LT;
.super Ljava/lang/Object;
.method public static foo(Z)I
    .registers 2
    if-eqz v1, :Lb
    const/4 v0, 1
    goto :Ljoin
    :Lb
    const/4 v0, 2
    :Ljoin
    return v0
.end method
"#,
    );
}

#[test]
fn ok_new_instance_init() {
    assert_ok(
        r#"
.class public LT;
.super Ljava/lang/Object;
.method public static foo()Ljava/lang/Object;
    .registers 1
    new-instance v0, Ljava/lang/Object;
    invoke-direct {v0}, Ljava/lang/Object;-><init>()V
    return-object v0
.end method
"#,
    );
}

#[test]
fn err_use_uninit_ref_before_init() {
    assert_err_contains(
        r#"
.class public LT;
.super Ljava/lang/Object;
.method public static foo()Ljava/lang/Object;
    .registers 1
    new-instance v0, Ljava/lang/Object;
    return-object v0
.end method
"#,
        "register-type",
    );
}

#[test]
fn ok_constructor_this() {
    assert_ok(
        r#"
.class public LT;
.super Ljava/lang/Object;
.method public constructor <init>()V
    .registers 1
    invoke-direct {v0}, Ljava/lang/Object;-><init>()V
    return-void
.end method
"#,
    );
}

#[test]
fn err_return_this_before_init() {
    assert_err_contains(
        r#"
.class public LT;
.super Ljava/lang/Object;
.method public constructor <init>()Ljava/lang/Object;
    .registers 1
    return-object v0
.end method
"#,
        "register-type",
    );
}

#[test]
fn ok_null_as_object() {
    assert_ok(
        r#"
.class public LT;
.super Ljava/lang/Object;
.method public static foo()Ljava/lang/String;
    .registers 1
    const/4 v0, 0
    return-object v0
.end method
"#,
    );
}

#[test]
fn ok_array_aget() {
    assert_ok(
        r#"
.class public LT;
.super Ljava/lang/Object;
.method public static foo([I)I
    .registers 3
    const/4 v1, 0
    aget v0, v2, v1
    return v0
.end method
"#,
    );
}

#[test]
fn err_aget_on_int() {
    assert_err_contains(
        r#"
.class public LT;
.super Ljava/lang/Object;
.method public static foo()I
    .registers 3
    const/4 v0, 5
    const/4 v1, 0
    aget v2, v0, v1
    return v2
.end method
"#,
        "aget array",
    );
}

#[test]
fn err_aput_wide_from_int() {
    assert_err_contains(
        r#"
.class public LT;
.super Ljava/lang/Object;
.method public static foo([J)V
    .registers 3
    const/4 v0, 1
    const/4 v1, 0
    aput-wide v0, v2, v1
    return-void
.end method
"#,
        "register-type",
    );
}

#[test]
fn ok_move_object() {
    assert_ok(
        r#"
.class public LT;
.super Ljava/lang/Object;
.method public static foo(Ljava/lang/String;)Ljava/lang/String;
    .registers 2
    move-object v0, v1
    return-object v0
.end method
"#,
    );
}

#[test]
fn err_move_object_from_int() {
    assert_err_contains(
        r#"
.class public LT;
.super Ljava/lang/Object;
.method public static foo()Ljava/lang/Object;
    .registers 2
    const/4 v1, 5
    move-object v0, v1
    return-object v0
.end method
"#,
        "move-object",
    );
}

#[test]
fn ok_binop_int() {
    assert_ok(
        r#"
.class public LT;
.super Ljava/lang/Object;
.method public static foo()I
    .registers 3
    const/4 v0, 1
    const/4 v1, 2
    add-int v2, v0, v1
    return v2
.end method
"#,
    );
}

#[test]
fn err_add_int_with_wide() {
    assert_err_contains(
        r#"
.class public LT;
.super Ljava/lang/Object;
.method public static foo()I
    .registers 3
    const-wide/16 v0, 1
    const/4 v2, 1
    add-int v2, v0, v2
    return v2
.end method
"#,
        "register-type",
    );
}

#[test]
fn ok_packed_switch_targets() {
    assert_ok(
        r#"
.class public LT;
.super Ljava/lang/Object;
.method public static foo(I)I
    .registers 2
    packed-switch v1, :Lpayload
    const/4 v0, 0
    goto :Lend
    :Lcase1
    const/4 v0, 1
    goto :Lend
    :Lcase2
    const/4 v0, 2
    :Lend
    return v0
    :Lpayload
    .packed-switch 0
        :Lcase1
        :Lcase2
    .end packed-switch
.end method
"#,
    );
}

#[test]
fn ok_instance_method_this() {
    assert_ok(
        r#"
.class public LT;
.super Ljava/lang/Object;
.method public foo()LT;
    .registers 1
    return-object v0
.end method
"#,
    );
}

#[test]
fn err_move_result_object_for_int_invoke() {
    assert_err_contains(
        r#"
.class public LT;
.super Ljava/lang/Object;
.method public static foo()Ljava/lang/Object;
    .registers 1
    invoke-static {}, LT;->bar()I
    move-result-object v0
    return-object v0
.end method
.method public static bar()I
    .registers 1
    const/4 v0, 0
    return v0
.end method
"#,
        "move-result-object",
    );
}

#[test]
fn ok_check_cast() {
    assert_ok(
        r#"
.class public LT;
.super Ljava/lang/Object;
.method public static foo(Ljava/lang/Object;)Ljava/lang/String;
    .registers 1
    check-cast v0, Ljava/lang/String;
    return-object v0
.end method
"#,
    );
}

#[test]
fn err_throw_int() {
    assert_err_contains(
        r#"
.class public LT;
.super Ljava/lang/Object;
.method public static foo()V
    .registers 1
    const/4 v0, 1
    throw v0
.end method
"#,
        "register-type",
    );
}

#[test]
fn ok_params_long() {
    assert_ok(
        r#"
.class public LT;
.super Ljava/lang/Object;
.method public static foo(J)J
    .registers 2
    return-wide v0
.end method
"#,
    );
}

#[test]
fn ok_if_eq_references() {
    assert_ok(
        r#"
.class public LT;
.super Ljava/lang/Object;
.method public static foo(Ljava/lang/Object;Ljava/lang/Object;)I
    .registers 3
    if-eq v1, v2, :Leq
    const/4 v0, 0
    return v0
    :Leq
    const/4 v0, 1
    return v0
.end method
"#,
    );
}

#[test]
fn err_if_lt_on_reference() {
    assert_err_contains(
        r#"
.class public LT;
.super Ljava/lang/Object;
.method public static foo(Ljava/lang/Object;Ljava/lang/Object;)I
    .registers 3
    if-lt v1, v2, :L
    const/4 v0, 0
    return v0
    :L
    const/4 v0, 1
    return v0
.end method
"#,
        "register-type",
    );
}

#[test]
fn err_if_eq_int_vs_ref() {
    assert_err_contains(
        r#"
.class public LT;
.super Ljava/lang/Object;
.method public static foo(Ljava/lang/Object;)I
    .registers 3
    const/4 v0, 1
    if-eq v0, v2, :L
    const/4 v1, 0
    return v1
    :L
    const/4 v1, 1
    return v1
.end method
"#,
        "incompatible",
    );
}

