//! Framework APK install / list / clean / publicize (Apktool parity).

mod embedded;

use std::fs;
use std::path::{Path, PathBuf};

use apkparser::ApkWriter;
use thiserror::Error;

pub use embedded::{embedded_android_framework_apk, ensure_embedded_framework};

const SPEC_PUBLIC: u32 = 0x4000_0000;
const RES_TABLE_TYPE: u16 = 0x0002;
const RES_TABLE_PACKAGE_TYPE: u16 = 0x0200;
const RES_TABLE_TYPE_SPEC_TYPE: u16 = 0x0202;

#[derive(Error, Debug)]
pub enum FrameworkError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Apk(#[from] apkparser::Error),
    #[error("framework error: {0}")]
    Framework(String),
}

pub type Result<T> = std::result::Result<T, FrameworkError>;

/// Options for framework operations (mirrors Apktool Config framework fields).
#[derive(Debug, Clone, Default)]
pub struct FrameworkOptions {
    /// Override framework storage directory (`-p` / `--frame-path`).
    pub frame_path: Option<PathBuf>,
    /// Optional tag suffix (`-t` / `--frame-tag`) → `{id}-{tag}.apk`.
    pub tag: Option<String>,
    /// When cleaning/listing, ignore tag filter (`-a` / `--all` style).
    pub all_tags: bool,
}

/// Resolve the default framework directory (Apktool-compatible paths).
pub fn default_framework_dir() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        #[cfg(target_os = "macos")]
        {
            return PathBuf::from(home).join("Library/apktool/framework");
        }
        #[cfg(target_os = "windows")]
        {
            if let Ok(local) = std::env::var("LOCALAPPDATA") {
                return PathBuf::from(local).join("apktool/framework");
            }
            return PathBuf::from(home).join("AppData/Local/apktool/framework");
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
                return PathBuf::from(xdg).join("apktool/framework");
            }
            return PathBuf::from(home).join(".local/share/apktool/framework");
        }
    }
    PathBuf::from("framework")
}

pub fn framework_directory(options: &FrameworkOptions) -> Result<PathBuf> {
    let dir = options
        .frame_path
        .clone()
        .unwrap_or_else(default_framework_dir);
    if dir.exists() && !dir.is_dir() {
        return Err(FrameworkError::Framework(format!(
            "framework path is not a directory: {}",
            dir.display()
        )));
    }
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn apk_suffix(tag: Option<&str>) -> String {
    match tag {
        Some(t) if !t.is_empty() => format!("-{t}.apk"),
        _ => ".apk".into(),
    }
}

/// Install a framework APK: extract `resources.arsc` (+ optional manifest), publicize, write `{id}[-tag].apk`.
pub fn install_framework(apk_path: &Path, options: &FrameworkOptions) -> Result<PathBuf> {
    let dir = framework_directory(options)?;
    let apk_bytes = fs::read(apk_path)?;
    let apk = apkparser::Apk::from_bytes(
        &apk_bytes,
        apkparser::ApkOptions::default().with_signature(false),
    )?;

    let mut arsc = apk
        .get_file("resources.arsc")
        .map_err(|_| FrameworkError::Framework(format!(
            "Could not find resources.arsc in file: {}",
            apk_path.display()
        )))?;
    publicize_resources_bytes(&mut arsc)?;

    let pkg_id = first_package_id(&arsc).ok_or_else(|| {
        FrameworkError::Framework("No packages in resources.arsc in file.".into())
    })?;

    let out_name = format!("{pkg_id}{}", apk_suffix(options.tag.as_deref()));
    let out_path = dir.join(&out_name);

    let mut writer = ApkWriter::new();
    writer.add_entry("resources.arsc", &arsc, false);
    if let Ok(manifest) = apk.get_file("AndroidManifest.xml") {
        writer.add_entry("AndroidManifest.xml", &manifest, false);
    }
    let out_bytes = writer.finish()?;
    fs::write(&out_path, out_bytes)?;
    Ok(out_path)
}

/// List installed framework APKs.
pub fn list_frameworks(options: &FrameworkOptions) -> Result<Vec<PathBuf>> {
    let dir = framework_directory(options)?;
    let suffix = if options.all_tags {
        ".apk".to_string()
    } else {
        apk_suffix(options.tag.as_deref())
    };
    let mut out = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_valid_framework_name(&name, &suffix, options.all_tags) {
            out.push(entry.path());
        }
    }
    out.sort();
    Ok(out)
}

/// Remove installed framework APKs matching the current tag (or all with `all_tags`).
pub fn clean_frameworks(options: &FrameworkOptions) -> Result<Vec<PathBuf>> {
    let files = list_frameworks(options)?;
    for path in &files {
        fs::remove_file(path)?;
    }
    Ok(files)
}

/// Publicize all entry specs in a standalone `resources.arsc` file (Apktool `pr`).
pub fn publicize_resources_file(arsc_path: &Path) -> Result<()> {
    let mut data = fs::read(arsc_path)?;
    publicize_resources_bytes(&mut data)?;
    fs::write(arsc_path, data)?;
    Ok(())
}

/// Set `SPEC_PUBLIC` on every `ResTable_typeSpec` entry flag.
pub fn publicize_resources_bytes(data: &mut [u8]) -> Result<()> {
    if data.len() < 12 {
        return Err(FrameworkError::Framework("resources.arsc too short".into()));
    }
    let table_type = u16::from_le_bytes([data[0], data[1]]);
    if table_type != RES_TABLE_TYPE {
        return Err(FrameworkError::Framework(format!(
            "not a resource table (type 0x{table_type:04x})"
        )));
    }
    let table_size = u32::from_le_bytes(data[4..8].try_into().unwrap()) as usize;
    let header_size = u16::from_le_bytes([data[2], data[3]]) as usize;
    let mut pos = header_size;
    let end = table_size.min(data.len());

    while pos + 8 <= end {
        let chunk_type = u16::from_le_bytes([data[pos], data[pos + 1]]);
        let chunk_header = u16::from_le_bytes([data[pos + 2], data[pos + 3]]) as usize;
        let chunk_size = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap()) as usize;
        if chunk_size < 8 || pos + chunk_size > end {
            break;
        }

        if chunk_type == RES_TABLE_TYPE_SPEC_TYPE {
            // ResTable_typeSpec: after header: id(1)+res0(1)+res1(2)+entryCount(4)
            if chunk_header + 8 <= chunk_size {
                let entry_count = u32::from_le_bytes(
                    data[pos + chunk_header + 4..pos + chunk_header + 8]
                        .try_into()
                        .unwrap(),
                ) as usize;
                let flags_off = pos + chunk_header + 8;
                for i in 0..entry_count {
                    let fo = flags_off + i * 4;
                    if fo + 4 > pos + chunk_size {
                        break;
                    }
                    let flags = u32::from_le_bytes(data[fo..fo + 4].try_into().unwrap());
                    data[fo..fo + 4].copy_from_slice(&(flags | SPEC_PUBLIC).to_le_bytes());
                }
            }
        } else if chunk_type == RES_TABLE_PACKAGE_TYPE {
            // Walk nested chunks inside the package for typeSpecs.
            publicize_nested_type_specs(data, pos, chunk_header, chunk_size)?;
        }

        pos += chunk_size;
    }
    Ok(())
}

fn publicize_nested_type_specs(
    data: &mut [u8],
    pkg_off: usize,
    pkg_header: usize,
    pkg_size: usize,
) -> Result<()> {
    let mut pos = pkg_off + pkg_header;
    let end = pkg_off + pkg_size;
    while pos + 8 <= end {
        let chunk_type = u16::from_le_bytes([data[pos], data[pos + 1]]);
        let chunk_header = u16::from_le_bytes([data[pos + 2], data[pos + 3]]) as usize;
        let chunk_size = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap()) as usize;
        if chunk_size < 8 || pos + chunk_size > end {
            break;
        }
        if chunk_type == RES_TABLE_TYPE_SPEC_TYPE && chunk_header + 8 <= chunk_size {
            let entry_count = u32::from_le_bytes(
                data[pos + chunk_header + 4..pos + chunk_header + 8]
                    .try_into()
                    .unwrap(),
            ) as usize;
            let flags_off = pos + chunk_header + 8;
            for i in 0..entry_count {
                let fo = flags_off + i * 4;
                if fo + 4 > pos + chunk_size {
                    break;
                }
                let flags = u32::from_le_bytes(data[fo..fo + 4].try_into().unwrap());
                data[fo..fo + 4].copy_from_slice(&(flags | SPEC_PUBLIC).to_le_bytes());
            }
        }
        pos += chunk_size;
    }
    Ok(())
}

/// Read the first package id from a resource table.
pub fn first_package_id(data: &[u8]) -> Option<u32> {
    if data.len() < 12 {
        return None;
    }
    let table_size = u32::from_le_bytes(data[4..8].try_into().ok()?) as usize;
    let header_size = u16::from_le_bytes([data[2], data[3]]) as usize;
    let mut pos = header_size;
    let end = table_size.min(data.len());
    while pos + 12 <= end {
        let chunk_type = u16::from_le_bytes([data[pos], data[pos + 1]]);
        let chunk_size = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().ok()?) as usize;
        if chunk_size < 12 || pos + chunk_size > end {
            break;
        }
        if chunk_type == RES_TABLE_PACKAGE_TYPE {
            let header = u16::from_le_bytes([data[pos + 2], data[pos + 3]]) as usize;
            if header >= 12 {
                return Some(u32::from_le_bytes(data[pos + 8..pos + 12].try_into().ok()?));
            }
        }
        pos += chunk_size;
    }
    None
}

fn is_valid_framework_name(file_name: &str, suffix: &str, ignore_tag: bool) -> bool {
    if !file_name.ends_with(suffix) && !ignore_tag {
        return false;
    }
    if ignore_tag {
        if !file_name.ends_with(".apk") {
            return false;
        }
        let stem = file_name.trim_end_matches(".apk");
        let id_part = stem.split('-').next().unwrap_or(stem);
        return !id_part.is_empty() && id_part.chars().all(|c| c.is_ascii_digit());
    }
    let len = file_name.len().saturating_sub(suffix.len());
    if len == 0 {
        return false;
    }
    file_name[..len].chars().all(|c| c.is_ascii_digit())
}

/// Resolve a framework APK by package id (with tag fallback, like Apktool).
/// For id=1, writes the embedded android framework jar when missing.
pub fn get_framework_apk(id: u32, options: &FrameworkOptions) -> Result<PathBuf> {
    let dir = framework_directory(options)?;
    let tagged = dir.join(format!("{id}{}", apk_suffix(options.tag.as_deref())));
    if tagged.is_file() {
        return Ok(tagged);
    }
    let plain = dir.join(format!("{id}.apk"));
    if plain.is_file() {
        return Ok(plain);
    }
    if id == 1 && options.tag.as_ref().map(|t| t.is_empty()).unwrap_or(true) {
        let path = ensure_embedded_framework(&dir)?;
        return Ok(path);
    }
    Err(FrameworkError::Framework(format!(
        "Could not find framework resources for package ID: {id}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suffix_formatting() {
        assert_eq!(apk_suffix(None), ".apk");
        assert_eq!(apk_suffix(Some("building")), "-building.apk");
    }

    #[test]
    fn valid_names() {
        assert!(is_valid_framework_name("2.apk", ".apk", false));
        assert!(is_valid_framework_name("2-building.apk", "-building.apk", false));
        assert!(!is_valid_framework_name("2-building.apk", ".apk", false));
        assert!(is_valid_framework_name("2-building.apk", ".apk", true));
    }

    #[test]
    fn get_framework_writes_embedded_id1() {
        let dir = tempfile::tempdir().unwrap();
        let opts = FrameworkOptions {
            frame_path: Some(dir.path().to_path_buf()),
            ..Default::default()
        };
        let path = get_framework_apk(1, &opts).unwrap();
        assert!(path.ends_with("1.apk"));
        assert!(path.is_file());
        let path2 = get_framework_apk(1, &opts).unwrap();
        assert_eq!(path, path2);
    }
}
