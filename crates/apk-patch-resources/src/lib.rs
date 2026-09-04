//! Resource table decode / build helpers (Phase 3).

mod aapt2;
mod build_arsc;
mod ninepatch;

use std::collections::BTreeMap;
use std::path::Path;

use axml_parser::{ARSCParser, AXMLPrinter, ArscBag, ArscResourceEntry};
use thiserror::Error;

pub use aapt2::{
    aapt2_compile, aapt2_link, find_aapt2, find_android_jar, Aapt2CompileOptions, Aapt2Error,
    Aapt2LinkOptions,
};
pub use build_arsc::{
    build_arsc_from_project, BuildArscError, BuildArscOptions, BuildArscResult,
};
pub use ninepatch::{
    decode_nine_patch_png, is_nine_patch_png, parse_nine_patch, NinePatchChunk, NinePatchError,
};

#[derive(Error, Debug)]
pub enum ResourceError {
    #[error(transparent)]
    Arsc(#[from] axml_parser::ResParserError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("resource error: {0}")]
    Resource(String),
}

pub type Result<T> = std::result::Result<T, ResourceError>;

/// How to resolve resource references when emitting values XML.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResResolveMode {
    /// Rewrite `@XXXXXXXX` → `@type/name` when known; leave string content alone.
    #[default]
    Default,
    /// Like default, then inline concrete string/scalar values for local refs when known.
    Greedy,
    /// Keep raw `@XXXXXXXX` / typed refs; do not rewrite.
    Lazy,
}

#[derive(Debug, Clone, Default)]
pub struct DecodeResOptions {
    pub keep_broken: bool,
    pub resolve_mode: ResResolveMode,
    /// When true, skip values that only have opaque raw dumps (`<0x.., type 0x..>`).
    pub ignore_raw_values: bool,
}

const BAG_KEY_ATTR_TYPE: u32 = 0x0100_0000;
const BAG_KEY_ARRAY_START: u32 = 0x0200_0000;
const BAG_KEY_PLURALS_START: u32 = 0x0100_0004;
const BAG_KEY_PLURALS_END: u32 = 0x0100_0009;

const PLURAL_Q: [&str; 6] = ["other", "zero", "one", "two", "few", "many"];

/// Decode `resources.arsc` into `res/values*/` XMLs + `overlayable.xml`.
pub fn decode_resources_arsc(
    arsc_bytes: &[u8],
    output_dir: &Path,
    options: &DecodeResOptions,
) -> Result<DecodeResResult> {
    let parser = ARSCParser::new(arsc_bytes)?;
    let res_dir = output_dir.join("res");
    std::fs::create_dir_all(&res_dir)?;

    let public_xml = emit_public_xml(&parser);
    let values_dir = res_dir.join("values");
    std::fs::create_dir_all(&values_dir)?;
    std::fs::write(values_dir.join("public.xml"), public_xml)?;

    emit_typed_values(&parser, &res_dir, options)?;

    if !parser.overlayables.is_empty() {
        let overlay = emit_overlayable_xml(&parser);
        std::fs::write(values_dir.join("overlayable.xml"), overlay)?;
    }

    let package_id = parser.resources.keys().next().map(|id| id >> 24);

    Ok(DecodeResResult {
        package_names: parser.get_packages_names(),
        package_id,
        resource_count: parser.resources.len(),
    })
}

/// Decode a binary AXML resource file (layout/menu/drawable XML) to text UTF-8.
pub fn decode_res_xml(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 8 {
        return None;
    }
    let chunk_type = u16::from_le_bytes([data[0], data[1]]);
    if chunk_type != 0x0003 {
        return None;
    }
    let printer = AXMLPrinter::new(data);
    if !printer.is_valid() {
        return None;
    }
    Some(printer.get_xml(true))
}

/// True when `rel` under `res/` is an XML resource that should be text-decoded.
pub fn should_decode_res_xml(rel: &str) -> bool {
    let lower = rel.to_ascii_lowercase();
    if !lower.ends_with(".xml") {
        return false;
    }
    let first = lower.split('/').next().unwrap_or("");
    !first.starts_with("values")
}

#[derive(Debug)]
pub struct DecodeResResult {
    pub package_names: Vec<String>,
    pub package_id: Option<u32>,
    pub resource_count: usize,
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn emit_public_xml(parser: &ARSCParser) -> String {
    let mut by_type: BTreeMap<&str, Vec<(u32, &str)>> = BTreeMap::new();
    for (id, entry) in &parser.resources {
        by_type
            .entry(entry.type_name.as_str())
            .or_default()
            .push((*id, entry.name.as_str()));
    }
    for list in by_type.values_mut() {
        list.sort_by_key(|(id, _)| *id);
    }

    let mut out = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<resources>\n");
    for (type_name, entries) in by_type {
        for (id, name) in entries {
            out.push_str(&format!(
                "    <public type=\"{}\" name=\"{}\" id=\"0x{:08x}\" />\n",
                xml_escape(type_name),
                xml_escape(name),
                id
            ));
        }
    }
    out.push_str("</resources>\n");
    out
}

fn is_values_type(type_name: &str) -> bool {
    matches!(
        type_name,
        "string"
            | "bool"
            | "color"
            | "dimen"
            | "integer"
            | "fraction"
            | "id"
            | "array"
            | "integer-array"
            | "string-array"
            | "plurals"
            | "style"
            | "attr"
            | "macro"
    )
}

fn values_file_for_type(type_name: &str) -> &'static str {
    match type_name {
        "string" => "strings.xml",
        "bool" => "bools.xml",
        "color" => "colors.xml",
        "dimen" => "dimens.xml",
        "integer" => "integers.xml",
        "fraction" => "fractions.xml",
        "id" => "ids.xml",
        "array" | "integer-array" | "string-array" => "arrays.xml",
        "plurals" => "plurals.xml",
        "style" => "styles.xml",
        "attr" => "attrs.xml",
        _ => "values_extra.xml",
    }
}

fn emit_typed_values(
    parser: &ARSCParser,
    res_dir: &Path,
    options: &DecodeResOptions,
) -> Result<()> {
    let mut files: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    let id_map = parser.id_to_java_name();
    let value_map = parser.id_to_string_value();

    for entry in &parser.all_entries {
        if !is_values_type(&entry.type_name) {
            continue;
        }
        let dir = entry.config.values_dir_name();
        let file = values_file_for_type(&entry.type_name).to_string();
        if let Some(block) = format_values_block(entry, &id_map, &value_map, options) {
            files.entry((dir, file)).or_default().push(block);
        }
    }

    for ((dir_name, file_name), mut lines) in files {
        lines.sort();
        lines.dedup();
        let dir = res_dir.join(&dir_name);
        std::fs::create_dir_all(&dir)?;
        let mut out = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<resources>\n");
        for line in lines {
            out.push_str(&line);
            if !line.ends_with('\n') {
                out.push('\n');
            }
        }
        out.push_str("</resources>\n");
        std::fs::write(dir.join(file_name), out)?;
    }
    Ok(())
}

fn format_values_block(
    entry: &ArscResourceEntry,
    id_map: &std::collections::HashMap<u32, String>,
    value_map: &std::collections::HashMap<u32, String>,
    options: &DecodeResOptions,
) -> Option<String> {
    let name = xml_escape(&entry.name);

    if entry.type_name == "id" {
        return Some(format!("    <item type=\"id\" name=\"{name}\" />"));
    }

    if let Some(bag) = &entry.bag {
        return format_bag_block(entry, bag, id_map, value_map, options);
    }

    let mut value = entry.value.clone()?;
    if options.ignore_raw_values && is_raw_dump(&value) {
        return None;
    }
    value = rewrite_value(&value, id_map, value_map, options.resolve_mode);
    let tag = match entry.type_name.as_str() {
        "integer-array" => "integer-array",
        "string-array" => "string-array",
        other => other,
    };
    Some(format!(
        "    <{tag} name=\"{name}\">{}</{tag}>",
        xml_escape(&value)
    ))
}

fn format_bag_block(
    entry: &ArscResourceEntry,
    bag: &ArscBag,
    id_map: &std::collections::HashMap<u32, String>,
    value_map: &std::collections::HashMap<u32, String>,
    options: &DecodeResOptions,
) -> Option<String> {
    let name = xml_escape(&entry.name);
    let ty = entry.type_name.as_str();

    if ty == "style" || (ty != "array" && ty != "plurals" && ty != "attr" && looks_like_style(bag))
    {
        return Some(emit_style(name, bag, id_map, value_map, options));
    }
    if ty == "plurals" || looks_like_plurals(bag) {
        return Some(emit_plurals(name, bag, id_map, value_map, options));
    }
    if ty == "array" || ty == "string-array" || ty == "integer-array" || looks_like_array(bag) {
        return Some(emit_array(ty, name, bag, id_map, value_map, options));
    }
    if ty == "attr" {
        return Some(emit_attr(name, bag, id_map, value_map, options));
    }
    // Fallback: treat as style-like
    Some(emit_style(name, bag, id_map, value_map, options))
}

fn looks_like_array(bag: &ArscBag) -> bool {
    bag.items
        .first()
        .map(|i| i.name_id == BAG_KEY_ARRAY_START || i.name_id >= BAG_KEY_ARRAY_START)
        .unwrap_or(false)
}

fn looks_like_plurals(bag: &ArscBag) -> bool {
    bag.items.iter().any(|i| {
        (BAG_KEY_PLURALS_START..=BAG_KEY_PLURALS_END).contains(&i.name_id)
    })
}

fn looks_like_style(bag: &ArscBag) -> bool {
    !looks_like_array(bag) && !looks_like_plurals(bag) && !bag.items.is_empty()
}

fn emit_style(
    name: String,
    bag: &ArscBag,
    id_map: &std::collections::HashMap<u32, String>,
    value_map: &std::collections::HashMap<u32, String>,
    options: &DecodeResOptions,
) -> String {
    let mut out = String::new();
    if bag.parent != 0 {
        let parent = rewrite_value(
            &format!("@{:08X}", bag.parent),
            id_map,
            value_map,
            options.resolve_mode,
        );
        out.push_str(&format!(
            "    <style name=\"{name}\" parent=\"{}\">\n",
            xml_escape(&parent)
        ));
    } else {
        out.push_str(&format!("    <style name=\"{name}\">\n"));
    }
    for item in &bag.items {
        let key = attr_ref(item.name_id, id_map, options.resolve_mode);
        let Some(mut val) = item.formatted.clone() else {
            continue;
        };
        if options.ignore_raw_values && is_raw_dump(&val) {
            continue;
        }
        val = rewrite_value(&val, id_map, value_map, options.resolve_mode);
        out.push_str(&format!(
            "        <item name=\"{}\">{}</item>\n",
            xml_escape(&key),
            xml_escape(&val)
        ));
    }
    out.push_str("    </style>");
    out
}

fn emit_plurals(
    name: String,
    bag: &ArscBag,
    id_map: &std::collections::HashMap<u32, String>,
    value_map: &std::collections::HashMap<u32, String>,
    options: &DecodeResOptions,
) -> String {
    let mut out = format!("    <plurals name=\"{name}\">\n");
    for item in &bag.items {
        if !(BAG_KEY_PLURALS_START..=BAG_KEY_PLURALS_END).contains(&item.name_id) {
            continue;
        }
        let q = PLURAL_Q[(item.name_id - BAG_KEY_PLURALS_START) as usize];
        let Some(mut val) = item.formatted.clone() else {
            continue;
        };
        if options.ignore_raw_values && is_raw_dump(&val) {
            continue;
        }
        val = rewrite_value(&val, id_map, value_map, options.resolve_mode);
        out.push_str(&format!(
            "        <item quantity=\"{q}\">{}</item>\n",
            xml_escape(&val)
        ));
    }
    out.push_str("    </plurals>");
    out
}

fn emit_array(
    type_name: &str,
    name: String,
    bag: &ArscBag,
    id_map: &std::collections::HashMap<u32, String>,
    value_map: &std::collections::HashMap<u32, String>,
    options: &DecodeResOptions,
) -> String {
    // Infer string-array vs integer-array from element types when type is generic "array".
    let tag = if type_name == "array" {
        let all_string = bag.items.iter().all(|i| {
            i.data_type == axml_parser::constants::TYPE_STRING
                || i.formatted
                    .as_ref()
                    .map(|v| !v.chars().all(|c| c.is_ascii_digit() || c == '-'))
                    .unwrap_or(false)
        });
        if all_string {
            "string-array"
        } else {
            "array"
        }
    } else if type_name == "integer-array" {
        "integer-array"
    } else if type_name == "string-array" {
        "string-array"
    } else {
        "array"
    };

    let mut out = format!("    <{tag} name=\"{name}\">\n");
    for item in &bag.items {
        let Some(mut val) = item.formatted.clone() else {
            continue;
        };
        if options.ignore_raw_values && is_raw_dump(&val) {
            continue;
        }
        val = rewrite_value(&val, id_map, value_map, options.resolve_mode);
        out.push_str(&format!("        <item>{}</item>\n", xml_escape(&val)));
    }
    out.push_str(&format!("    </{tag}>"));
    out
}

fn emit_attr(
    name: String,
    bag: &ArscBag,
    id_map: &std::collections::HashMap<u32, String>,
    value_map: &std::collections::HashMap<u32, String>,
    options: &DecodeResOptions,
) -> String {
    let mut format_bits: Option<u32> = None;
    let mut enums: Vec<(String, String)> = Vec::new();
    let mut flags: Vec<(String, String)> = Vec::new();

    for item in &bag.items {
        if item.name_id == BAG_KEY_ATTR_TYPE {
            format_bits = Some(item.data);
            continue;
        }
        // Enum/flag entries use high attr ids as keys with TYPE_INT value.
        if item.name_id > BAG_KEY_ATTR_TYPE {
            let ename = attr_ref(item.name_id, id_map, options.resolve_mode);
            let val = item
                .formatted
                .clone()
                .unwrap_or_else(|| item.data.to_string());
            // Heuristic: if format includes flags (0x00000010) treat as flag
            let is_flag = format_bits.map(|f| f & 0x0002_0000 != 0).unwrap_or(false);
            if is_flag {
                flags.push((ename, val));
            } else {
                enums.push((ename, val));
            }
        }
    }

    let format_attr = format_bits
        .map(|f| format!(" format=\"{}\"", xml_escape(&attr_format_string(f))))
        .unwrap_or_default();

    if enums.is_empty() && flags.is_empty() {
        return format!("    <attr name=\"{name}\"{format_attr} />");
    }

    let mut out = format!("    <attr name=\"{name}\"{format_attr}>\n");
    for (n, v) in enums {
        let n = strip_attr_name(&n);
        out.push_str(&format!(
            "        <enum name=\"{}\" value=\"{}\" />\n",
            xml_escape(&n),
            xml_escape(&rewrite_value(&v, id_map, value_map, options.resolve_mode))
        ));
    }
    for (n, v) in flags {
        let n = strip_attr_name(&n);
        out.push_str(&format!(
            "        <flag name=\"{}\" value=\"{}\" />\n",
            xml_escape(&n),
            xml_escape(&rewrite_value(&v, id_map, value_map, options.resolve_mode))
        ));
    }
    out.push_str("    </attr>");
    out
}

fn strip_attr_name(s: &str) -> String {
    s.trim_start_matches('@')
        .trim_start_matches("android:")
        .rsplit('/')
        .next()
        .unwrap_or(s)
        .to_string()
}

fn attr_format_string(bits: u32) -> String {
    // android.content.res.Attribute format bitmasks.
    let mut parts = Vec::new();
    if bits & 0x0000_0001 != 0 {
        parts.push("reference");
    }
    if bits & 0x0000_0002 != 0 {
        parts.push("string");
    }
    if bits & 0x0000_0004 != 0 {
        parts.push("integer");
    }
    if bits & 0x0000_0008 != 0 {
        parts.push("boolean");
    }
    if bits & 0x0000_0010 != 0 {
        parts.push("color");
    }
    if bits & 0x0000_0020 != 0 {
        parts.push("float");
    }
    if bits & 0x0000_0040 != 0 {
        parts.push("dimension");
    }
    if bits & 0x0000_0080 != 0 {
        parts.push("fraction");
    }
    if bits & 0x0001_0000 != 0 {
        parts.push("enum");
    }
    if bits & 0x0002_0000 != 0 {
        parts.push("flags");
    }
    if parts.is_empty() {
        format!("0x{bits:08x}")
    } else {
        parts.join("|")
    }
}

fn attr_ref(
    id: u32,
    id_map: &std::collections::HashMap<u32, String>,
    mode: ResResolveMode,
) -> String {
    if mode == ResResolveMode::Lazy {
        return format!("@{:08X}", id);
    }
    if let Some(java) = id_map.get(&id) {
        if let Some(rest) = java.strip_prefix("android.R.") {
            let mut parts = rest.splitn(2, '.');
            let ty = parts.next().unwrap_or("attr");
            let name = parts.next().unwrap_or("unknown");
            return format!("android:{ty}/{name}");
        }
        if let Some(rest) = java.strip_prefix("R.") {
            let mut parts = rest.splitn(2, '.');
            let _ty = parts.next();
            let name = parts.next().unwrap_or("unknown");
            return name.to_string();
        }
    }
    // Platform attr ids often live in 0x0101xxxx without local map.
    if (id >> 16) == 0x0101 {
        if let Some(n) = axml_parser::public::get_attr_name(id) {
            return format!("android:attr/{n}");
        }
    }
    format!("@{:08X}", id)
}

fn is_raw_dump(value: &str) -> bool {
    value.starts_with('<') && value.contains("type 0x")
}

fn rewrite_value(
    value: &str,
    id_map: &std::collections::HashMap<u32, String>,
    value_map: &std::collections::HashMap<u32, String>,
    mode: ResResolveMode,
) -> String {
    if mode == ResResolveMode::Lazy {
        return value.to_string();
    }
    if value.len() == 9 && value.starts_with('@') {
        if let Ok(id) = u32::from_str_radix(&value[1..], 16) {
            if mode == ResResolveMode::Greedy {
                if let Some(concrete) = value_map.get(&id) {
                    if !concrete.starts_with('@') && !is_raw_dump(concrete) {
                        return concrete.clone();
                    }
                }
            }
            if let Some(java) = id_map.get(&id) {
                if let Some(rest) = java.strip_prefix("android.R.") {
                    let mut parts = rest.splitn(2, '.');
                    let ty = parts.next().unwrap_or("id");
                    let name = parts.next().unwrap_or("unknown");
                    return format!("@android:{ty}/{name}");
                }
                if let Some(rest) = java.strip_prefix("R.") {
                    let mut parts = rest.splitn(2, '.');
                    let ty = parts.next().unwrap_or("id");
                    let name = parts.next().unwrap_or("unknown");
                    return format!("@{ty}/{name}");
                }
            }
        }
    }
    value.to_string()
}

fn emit_overlayable_xml(parser: &ARSCParser) -> String {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<overlayable>\n");
    for group in &parser.overlayables {
        out.push_str(&format!(
            "    <overlayable name=\"{}\" actor=\"{}\">\n",
            xml_escape(&group.name),
            xml_escape(&group.actor)
        ));
        for pol in &group.policies {
            out.push_str(&format!(
                "        <policy flags=\"0x{:08x}\">\n",
                pol.flags
            ));
            for id in &pol.entries {
                if let Some(e) = parser.resources.get(id) {
                    out.push_str(&format!(
                        "            <item type=\"{}\" name=\"{}\" />\n",
                        xml_escape(&e.type_name),
                        xml_escape(&e.name)
                    ));
                } else {
                    out.push_str(&format!("            <item id=\"0x{id:08x}\" />\n"));
                }
            }
            out.push_str("        </policy>\n");
        }
        out.push_str("    </overlayable>\n");
    }
    out.push_str("</overlayable>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_escape_basic() {
        assert_eq!(xml_escape("a&b"), "a&amp;b");
    }

    #[test]
    fn should_decode_layout_xml() {
        assert!(should_decode_res_xml("layout/activity_main.xml"));
        assert!(!should_decode_res_xml("values/strings.xml"));
        assert!(!should_decode_res_xml("drawable/icon.png"));
    }

    #[test]
    fn lazy_keeps_raw_refs() {
        let mut id_map = std::collections::HashMap::new();
        id_map.insert(0x7f0b0001, "R.string.hello".into());
        let value_map = std::collections::HashMap::new();
        assert_eq!(
            rewrite_value("@7F0B0001", &id_map, &value_map, ResResolveMode::Lazy),
            "@7F0B0001"
        );
        assert_eq!(
            rewrite_value("@7F0B0001", &id_map, &value_map, ResResolveMode::Default),
            "@string/hello"
        );
    }
}
