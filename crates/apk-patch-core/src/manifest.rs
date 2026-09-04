//! AndroidManifest.xml decode/encode helpers (Phase 2).

use axml_parser::{encode_xml, AXMLPrinter};

use crate::decode::DecodeError;

/// Decode binary AXML manifest bytes to pretty-printed UTF-8 XML text.
pub fn decode_manifest_to_xml(data: &[u8]) -> Result<Vec<u8>, DecodeError> {
    let printer = AXMLPrinter::new(data);
    if !printer.is_valid() {
        return Err(DecodeError::Decode(
            "invalid or unsupported AndroidManifest.xml".into(),
        ));
    }
    Ok(printer.get_xml(true))
}

/// Encode text XML (or pass through binary AXML) to binary AndroidManifest.xml.
pub fn encode_manifest_to_axml(data: &[u8]) -> Result<Vec<u8>, DecodeError> {
    if looks_like_binary_axml(data) {
        return Ok(data.to_vec());
    }
    let text = std::str::from_utf8(data)
        .map_err(|e| DecodeError::Decode(format!("manifest is not UTF-8: {e}")))?;
    let normalized = normalize_clark_android_attrs(text);
    encode_xml(normalized.as_bytes())
        .map_err(|e| DecodeError::Decode(format!("AXML encode failed: {e}")))
}

/// `{http://schemas.android.com/apk/res/android}foo` → `android:foo` for re-encode.
fn normalize_clark_android_attrs(xml: &str) -> String {
    xml.replace(
        "{http://schemas.android.com/apk/res/android}",
        "android:",
    )
}

/// True if `data` looks like a binary AXML / RES_XML chunk (not text XML).
pub fn looks_like_binary_axml(data: &[u8]) -> bool {
    if data.len() < 8 {
        return false;
    }
    let chunk_type = u16::from_le_bytes([data[0], data[1]]);
    // RES_XML_TYPE = 0x0003
    chunk_type == 0x0003
}
