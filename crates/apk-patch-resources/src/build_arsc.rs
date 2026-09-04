//! Build `resources.arsc` from a decoded apk-patch project tree (pure Rust).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use axml_parser::{
    build_arsc, ArscBuildConfig, ArscBuildEntry, ArscBuildInput, ArscBuildType, ArscBuildValue,
};
use thiserror::Error;
use walkdir::WalkDir;

use crate::ResourceError;

#[derive(Error, Debug)]
pub enum BuildArscError {
    #[error(transparent)]
    Resource(#[from] ResourceError),
    #[error(transparent)]
    Arsc(#[from] axml_parser::ResParserError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("build arsc: {0}")]
    Build(String),
}

pub type Result<T> = std::result::Result<T, BuildArscError>;

#[derive(Debug, Clone, Default)]
pub struct BuildArscOptions {
    pub package_id: Option<u32>,
    pub package_name: Option<String>,
}

#[derive(Debug)]
pub struct BuildArscResult {
    pub arsc: Vec<u8>,
    pub package_id: u32,
    pub package_name: String,
    pub entry_count: usize,
}

/// Build `resources.arsc` from `res/values*/` + `public.xml` + file-based `res/` entries.
pub fn build_arsc_from_project(project: &Path, options: &BuildArscOptions) -> Result<BuildArscResult> {
    let res_dir = project.join("res");
    if !res_dir.is_dir() {
        return Err(BuildArscError::Build("missing res/ directory".into()));
    }

    let public_path = res_dir.join("values/public.xml");
    let public = if public_path.is_file() {
        parse_public_xml(&std::fs::read_to_string(&public_path)?)?
    } else {
        Vec::new()
    };

    let package_id = options
        .package_id
        .or_else(|| public.first().map(|(id, _, _)| id >> 24))
        .unwrap_or(0x7f);
    let package_name = options
        .package_name
        .clone()
        .unwrap_or_else(|| "app".into());

    // type → (type_id, entry_index → (name, optional value, public))
    let mut by_type: BTreeMap<String, (u8, BTreeMap<u16, (String, Option<ArscBuildValue>, bool)>)> =
        BTreeMap::new();

    for (id, type_name, name) in &public {
        if id >> 24 != package_id {
            continue;
        }
        let type_id = ((id >> 16) & 0xff) as u8;
        let entry_index = (id & 0xffff) as u16;
        let slot = by_type
            .entry(type_name.clone())
            .or_insert_with(|| (type_id, BTreeMap::new()));
        if slot.0 == 0 {
            slot.0 = type_id;
        }
        slot.1
            .insert(entry_index, (name.clone(), None, true));
    }

    // Overlay scalar values from values*/ XMLs.
    for values_dir in list_values_dirs(&res_dir)? {
        for entry in std::fs::read_dir(&values_dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let fname = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if fname == "public.xml" || fname == "overlayable.xml" {
                continue;
            }
            if !fname.ends_with(".xml") {
                continue;
            }
            let xml = std::fs::read_to_string(&path)?;
            for (type_name, name, value) in parse_values_xml(&xml)? {
                let next_id = by_type
                    .values()
                    .map(|(id, _)| *id)
                    .max()
                    .unwrap_or(0)
                    .saturating_add(1)
                    .max(1);
                if let Some((_, map)) = by_type.get_mut(&type_name) {
                    if let Some((_, slot, _)) = map.values_mut().find(|(n, _, _)| n == &name) {
                        *slot = Some(value);
                        continue;
                    }
                    let next = map.keys().next_back().map(|k| k + 1).unwrap_or(0);
                    map.insert(next, (name, Some(value), true));
                } else {
                    let mut map = BTreeMap::new();
                    map.insert(0, (name, Some(value), true));
                    by_type.insert(type_name, (next_id, map));
                }
            }
        }
    }

    // File-based resources: path string values.
    for (type_name, apk_path, name) in scan_file_resources(&res_dir)? {
        let next_id = by_type
            .values()
            .map(|(id, _)| *id)
            .max()
            .unwrap_or(0)
            .saturating_add(1)
            .max(1);
        if let Some((_, map)) = by_type.get_mut(&type_name) {
            if let Some((_, v, _)) = map.values_mut().find(|(n, _, _)| n == &name) {
                if v.is_none() {
                    *v = Some(ArscBuildValue::String(apk_path));
                }
                continue;
            }
            let next = map.keys().next_back().map(|k| k + 1).unwrap_or(0);
            map.insert(next, (name, Some(ArscBuildValue::String(apk_path)), true));
        } else {
            let mut map = BTreeMap::new();
            map.insert(0, (name, Some(ArscBuildValue::String(apk_path)), true));
            by_type.insert(type_name, (next_id, map));
        }
    }

    let mut types = Vec::new();
    let mut entry_count = 0usize;
    for (type_name, (type_id, entries)) in by_type {
        let mut build_entries = Vec::new();
        for (entry_index, (name, value, public)) in entries {
            let value = if type_name == "id" {
                ArscBuildValue::Bool(false)
            } else {
                value.unwrap_or(ArscBuildValue::Null)
            };
            build_entries.push(ArscBuildEntry {
                entry_index,
                name,
                value,
                public,
            });
            entry_count += 1;
        }
        types.push(ArscBuildType {
            name: type_name,
            type_id,
            configs: vec![ArscBuildConfig {
                qualifier: String::new(),
                raw: None,
                entries: build_entries,
            }],
        });
    }

    if types.is_empty() {
        return Err(BuildArscError::Build(
            "no resources found to encode (need public.xml and/or values*)".into(),
        ));
    }

    let input = ArscBuildInput {
        package_id,
        package_name: package_name.clone(),
        types,
    };
    let arsc = build_arsc(&input)?;
    Ok(BuildArscResult {
        arsc,
        package_id,
        package_name,
        entry_count,
    })
}

fn list_values_dirs(res_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut dirs = Vec::new();
    for entry in std::fs::read_dir(res_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type()?.is_dir() && name.starts_with("values") {
            dirs.push(entry.path());
        }
    }
    dirs.sort();
    Ok(dirs)
}

/// `(id, type, name)` from public.xml.
fn parse_public_xml(xml: &str) -> Result<Vec<(u32, String, String)>> {
    let mut out = Vec::new();
    for line in xml.lines() {
        let line = line.trim();
        if !line.starts_with("<public ") {
            continue;
        }
        let type_name = attr(line, "type").ok_or_else(|| {
            BuildArscError::Build(format!("public.xml missing type: {line}"))
        })?;
        let name = attr(line, "name").ok_or_else(|| {
            BuildArscError::Build(format!("public.xml missing name: {line}"))
        })?;
        let id_str = attr(line, "id").ok_or_else(|| {
            BuildArscError::Build(format!("public.xml missing id: {line}"))
        })?;
        let id = parse_res_id(&id_str)?;
        out.push((id, type_name, name));
    }
    Ok(out)
}

fn parse_res_id(s: &str) -> Result<u32> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16)
            .map_err(|e| BuildArscError::Build(format!("bad id {s}: {e}")))
    } else {
        s.parse::<u32>()
            .map_err(|e| BuildArscError::Build(format!("bad id {s}: {e}")))
    }
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let key = format!("{name}=\"");
    let start = tag.find(&key)? + key.len();
    let end = tag[start..].find('"')? + start;
    Some(tag[start..end].to_string())
}

/// Parse simple values XML → (type, name, value).
fn parse_values_xml(xml: &str) -> Result<Vec<(String, String, ArscBuildValue)>> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find('<') {
        rest = &rest[start..];
        if rest.starts_with("<!--") {
            if let Some(end) = rest.find("-->") {
                rest = &rest[end + 3..];
                continue;
            }
            break;
        }
        if rest.starts_with("</") || rest.starts_with("<?") || rest.starts_with("<resources") {
            if let Some(end) = rest.find('>') {
                rest = &rest[end + 1..];
                continue;
            }
            break;
        }
        // <tag ...>content</tag> or <tag ... />
        let end_gt = match rest.find('>') {
            Some(i) => i,
            None => break,
        };
        let open = &rest[1..end_gt];
        let self_closing = open.trim_end().ends_with('/');
        let open = open.trim_end().trim_end_matches('/').trim_end();
        let tag_name = open.split_whitespace().next().unwrap_or("").to_string();
        if tag_name.is_empty() || tag_name == "item" && !open.contains("type=\"id\"") {
            // skip nested items for now except top-level id items handled below
        }
        let name = match attr(&format!("<{open}>"), "name") {
            Some(n) => n,
            None => {
                rest = &rest[end_gt + 1..];
                continue;
            }
        };

        if self_closing || open.contains("type=\"id\"") {
            if tag_name == "item" || open.contains("type=\"id\"") {
                out.push(("id".into(), name, ArscBuildValue::Bool(false)));
            }
            rest = &rest[end_gt + 1..];
            continue;
        }

        let after = &rest[end_gt + 1..];
        let close = format!("</{tag_name}>");
        let Some(close_at) = after.find(&close) else {
            rest = after;
            continue;
        };
        let content = after[..close_at].trim();
        let type_name = match tag_name.as_str() {
            "string" => "string".to_string(),
            "bool" => "bool".to_string(),
            "color" => "color".to_string(),
            "dimen" => "dimen".to_string(),
            "integer" => "integer".to_string(),
            "fraction" => "fraction".to_string(),
            "item" => attr(&format!("<{open}>"), "type").unwrap_or_else(|| "id".into()),
            other => other.to_string(),
        };
        // Skip complex bag containers for now (style/array/plurals/attr with children).
        if matches!(
            tag_name.as_str(),
            "style" | "array" | "string-array" | "integer-array" | "plurals" | "attr"
        ) && content.contains('<')
        {
            rest = &after[close_at + close.len()..];
            continue;
        }
        let value = parse_scalar_value(&type_name, content);
        out.push((type_name, name, value));
        rest = &after[close_at + close.len()..];
    }
    Ok(out)
}

fn parse_scalar_value(type_name: &str, content: &str) -> ArscBuildValue {
    let c = html_unescape(content);
    if c.starts_with('@') {
        // @string/foo or @0x7f0b0001 or @7F0B0001
        if let Some(hex) = c.strip_prefix("@0x").or_else(|| c.strip_prefix("@0X")) {
            if let Ok(id) = u32::from_str_radix(hex, 16) {
                return ArscBuildValue::Reference(id);
            }
        }
        if c.len() == 9 && c.as_bytes()[1].is_ascii_hexdigit() {
            if let Ok(id) = u32::from_str_radix(&c[1..], 16) {
                return ArscBuildValue::Reference(id);
            }
        }
        // Unresolved symbolic ref — store as string for now.
        return ArscBuildValue::String(c);
    }
    match type_name {
        "bool" => ArscBuildValue::Bool(c == "true"),
        "integer" => {
            if let Some(hex) = c.strip_prefix("0x") {
                ArscBuildValue::IntHex(u32::from_str_radix(hex, 16).unwrap_or(0))
            } else {
                ArscBuildValue::IntDec(c.parse().unwrap_or(0))
            }
        }
        "color" => {
            let hex = c.trim_start_matches('#');
            let n = match hex.len() {
                8 => u32::from_str_radix(hex, 16).unwrap_or(0),
                6 => 0xff00_0000 | u32::from_str_radix(hex, 16).unwrap_or(0),
                _ => 0,
            };
            ArscBuildValue::ColorArgb8(n)
        }
        "dimen" | "fraction" => ArscBuildValue::String(c), // keep as string; full complex encode later
        _ => ArscBuildValue::String(c),
    }
}

fn html_unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

/// File-based resources under res/ (not values*).
fn scan_file_resources(res_dir: &Path) -> Result<Vec<(String, String, String)>> {
    let mut out = Vec::new();
    for entry in WalkDir::new(res_dir).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(res_dir)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let mut parts = rel.splitn(2, '/');
        let folder = parts.next().unwrap_or("");
        let file = parts.next().unwrap_or("");
        if folder.is_empty() || file.is_empty() {
            continue;
        }
        if folder.starts_with("values") {
            continue;
        }
        // drawable-hdpi → type drawable
        let type_name = folder.split('-').next().unwrap_or(folder).to_string();
        let name = Path::new(file)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(file)
            .trim_end_matches(".9")
            .to_string();
        let apk_path = format!("res/{rel}");
        out.push((type_name, apk_path, name));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_public_line() {
        let xml = r#"<?xml version="1.0"?>
<resources>
    <public type="string" name="app_name" id="0x7f0b0001" />
</resources>
"#;
        let v = parse_public_xml(xml).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].1, "string");
        assert_eq!(v[0].2, "app_name");
        assert_eq!(v[0].0, 0x7f0b0001);
    }

    #[test]
    fn parse_string_value() {
        let xml = r#"<?xml version="1.0"?>
<resources>
    <string name="app_name">Hello</string>
    <bool name="flag">true</bool>
</resources>
"#;
        let v = parse_values_xml(xml).unwrap();
        assert!(v.iter().any(|(t, n, _)| t == "string" && n == "app_name"));
        assert!(v.iter().any(|(t, n, _)| t == "bool" && n == "flag"));
    }
}
