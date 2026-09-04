//! Minimal embedded Android framework (package id=1) for decode fallback.

use std::io::Write;

use apkparser::ApkWriter;
use axml_parser::encode_xml;

/// Build a minimal framework APK with `resources.arsc` package id=1 named `android`.
pub fn embedded_android_framework_apk() -> Vec<u8> {
    let arsc = build_minimal_android_arsc();
    let manifest_xml = br#"<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android" package="android">
</manifest>
"#;
    let manifest = encode_xml(manifest_xml).unwrap_or_else(|_| manifest_xml.to_vec());
    let mut writer = ApkWriter::new();
    writer.add_entry("AndroidManifest.xml", &manifest, false);
    writer.add_entry("resources.arsc", &arsc, false);
    writer
        .finish()
        .expect("embedded framework zip must build")
}

/// Ensure `1.apk` exists under the framework directory, writing the embedded jar if missing.
pub fn ensure_embedded_framework(dir: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
    let path = dir.join("1.apk");
    if path.is_file() {
        return Ok(path);
    }
    std::fs::create_dir_all(dir)?;
    let bytes = embedded_android_framework_apk();
    let mut f = std::fs::File::create(&path)?;
    f.write_all(&bytes)?;
    Ok(path)
}

fn build_minimal_android_arsc() -> Vec<u8> {
    // ResTable + empty global string pool + package id=1 "android" with empty type/key pools.
    let mut out = Vec::new();

    let global_pool = string_pool(&[]);
    let type_pool = string_pool(&[]);
    let key_pool = string_pool(&[]);

    // Package header size: 288 (standard AOSP)
    let pkg_header_size: u16 = 288;
    let pkg_size = pkg_header_size as u32 + type_pool.len() as u32 + key_pool.len() as u32;

    let mut package = Vec::new();
    // chunk header
    package.extend_from_slice(&0x0200u16.to_le_bytes()); // RES_TABLE_PACKAGE_TYPE
    package.extend_from_slice(&pkg_header_size.to_le_bytes());
    package.extend_from_slice(&pkg_size.to_le_bytes());
    package.extend_from_slice(&1u32.to_le_bytes()); // id
    // name: utf-16 "android" + NUL, padded to 256 bytes
    let mut name = vec![0u8; 256];
    for (i, ch) in "android".encode_utf16().enumerate() {
        let b = ch.to_le_bytes();
        name[i * 2] = b[0];
        name[i * 2 + 1] = b[1];
    }
    package.extend_from_slice(&name);
    // typeStrings offset (from start of package chunk)
    package.extend_from_slice(&(pkg_header_size as u32).to_le_bytes());
    package.extend_from_slice(&0u32.to_le_bytes()); // lastPublicType
    // keyStrings offset
    package.extend_from_slice(&(pkg_header_size as u32 + type_pool.len() as u32).to_le_bytes());
    package.extend_from_slice(&0u32.to_le_bytes()); // lastPublicKey
    // typeIdOffset (optional field in newer headers) — pad remaining header to 288
    while package.len() < pkg_header_size as usize {
        package.push(0);
    }
    package.extend_from_slice(&type_pool);
    package.extend_from_slice(&key_pool);

    let table_header_size: u16 = 12;
    let table_size = table_header_size as u32 + global_pool.len() as u32 + package.len() as u32;

    out.extend_from_slice(&0x0002u16.to_le_bytes()); // RES_TABLE_TYPE
    out.extend_from_slice(&table_header_size.to_le_bytes());
    out.extend_from_slice(&table_size.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes()); // packageCount
    out.extend_from_slice(&global_pool);
    out.extend_from_slice(&package);
    out
}

fn string_pool(strings: &[&str]) -> Vec<u8> {
    // UTF-16 string pool
    let header_size: u16 = 28;
    let string_count = strings.len() as u32;
    let mut offsets = Vec::with_capacity(strings.len());
    let mut data = Vec::new();
    for s in strings {
        offsets.push(data.len() as u32);
        let utf16: Vec<u16> = s.encode_utf16().collect();
        let char_len = utf16.len() as u16;
        data.extend_from_slice(&char_len.to_le_bytes());
        for c in utf16 {
            data.extend_from_slice(&c.to_le_bytes());
        }
        data.extend_from_slice(&0u16.to_le_bytes()); // NUL
    }
    // pad data to 4 bytes
    while data.len() % 4 != 0 {
        data.push(0);
    }
    let strings_start = header_size as u32 + string_count * 4;
    let size = strings_start + data.len() as u32;

    let mut out = Vec::new();
    out.extend_from_slice(&0x0001u16.to_le_bytes()); // RES_STRING_POOL_TYPE
    out.extend_from_slice(&header_size.to_le_bytes());
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&string_count.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // styleCount
    out.extend_from_slice(&0u32.to_le_bytes()); // flags (UTF-16)
    out.extend_from_slice(&strings_start.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // stylesStart
    for off in offsets {
        out.extend_from_slice(&off.to_le_bytes());
    }
    out.extend_from_slice(&data);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::first_package_id;

    #[test]
    fn embedded_has_package_id_one() {
        let apk = embedded_android_framework_apk();
        assert!(!apk.is_empty());
        let parsed = apkparser::Apk::from_bytes(
            &apk,
            apkparser::ApkOptions::default().with_signature(false),
        )
        .unwrap();
        let arsc = parsed.get_file("resources.arsc").unwrap();
        assert_eq!(first_package_id(&arsc), Some(1));
    }
}
