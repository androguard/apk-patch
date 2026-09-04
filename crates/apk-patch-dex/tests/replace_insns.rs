//! Integration test for variable-size code_item replacement on real DEX.

use dex_parser::{replace_code_insns, DexFile, DexHelper};

#[test]
fn replace_test_varargs_method() {
    let dex_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../apk-parser/tests/data/APK/TestActivity.apk");
    if !dex_path.is_file() {
        return;
    }
    let apk = std::fs::read(&dex_path).unwrap();
    let zip = apkparser::Apk::from_bytes(&apk, apkparser::ApkOptions::default()).unwrap();
    let original = zip.get_file("classes.dex").unwrap().to_vec();

    let parsed = DexFile::parse(&original).unwrap();
    let helper = DexHelper::from_dex(&parsed);
    let method = helper
        .methods()
        .find_map(|m| {
            let m = m.ok()?;
            if m.info.name == "testVarArgs" && m.info.class.contains("TestActivity") {
                Some(m)
            } else {
                None
            }
        })
        .expect("testVarArgs");

    let code_off = method.code_off;
    let mut dex = original;
    replace_code_insns(&mut dex, code_off, &[0x13, 0x00, 0x00, 0x00]).unwrap();
    assert_eq!(
        dex.len() as u32,
        u32::from_le_bytes(dex[32..36].try_into().unwrap()),
        "file_size header must match buffer length"
    );
    let reparsed = DexFile::parse(&dex).expect("rebuilt dex must parse");
    let updated = reparsed.get_code_item(code_off).unwrap();
    assert_eq!(updated.insns_size, 2);
}
