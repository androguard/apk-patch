//! Apktool-compatible project directory layout.

use std::path::{Path, PathBuf};

use thiserror::Error;
use walkdir::WalkDir;

use apk_patch_meta::ApkToolMeta;

#[derive(Error, Debug)]
pub enum ProjectError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("project error: {0}")]
    Project(String),
}

pub type Result<T> = std::result::Result<T, ProjectError>;

/// How an APK ZIP entry is classified on decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Manifest,
    ResourceTable,
    Dex,
    Res,
    Asset,
    NativeLib,
    Original,
    Unknown,
}

/// Destination for a decoded entry relative to the project root.
#[derive(Debug, Clone)]
pub enum DecodeTarget {
    Root(String),
    Res(String),
    Assets(String),
    Lib(String),
    Original(String),
    Unknown(String),
}

pub fn classify_entry(name: &str) -> EntryKind {
    if name == "AndroidManifest.xml" {
        return EntryKind::Manifest;
    }
    if name == "resources.arsc" {
        return EntryKind::ResourceTable;
    }
    if is_dex_entry(name) {
        return EntryKind::Dex;
    }
    if name.starts_with("res/") {
        return EntryKind::Res;
    }
    if name.starts_with("assets/") {
        return EntryKind::Asset;
    }
    if name.starts_with("lib/") {
        return EntryKind::NativeLib;
    }
    if name.starts_with("META-INF/") || name == "stamp-cert-sha256" {
        return EntryKind::Original;
    }
    EntryKind::Unknown
}

pub fn decode_target(name: &str) -> DecodeTarget {
    match classify_entry(name) {
        EntryKind::Manifest => DecodeTarget::Original("AndroidManifest.xml".into()),
        EntryKind::ResourceTable | EntryKind::Dex => DecodeTarget::Root(name.to_string()),
        EntryKind::Res => {
            DecodeTarget::Res(name.strip_prefix("res/").unwrap_or(name).into())
        }
        EntryKind::Asset => {
            DecodeTarget::Assets(name.strip_prefix("assets/").unwrap_or(name).into())
        }
        EntryKind::NativeLib => {
            DecodeTarget::Lib(name.strip_prefix("lib/").unwrap_or(name).into())
        }
        EntryKind::Original => {
            if name.starts_with("META-INF/") {
                DecodeTarget::Original(name.to_string())
            } else {
                DecodeTarget::Original(name.to_string())
            }
        }
        EntryKind::Unknown => {
            DecodeTarget::Unknown(name.to_string())
        }
    }
}

pub fn is_dex_entry(name: &str) -> bool {
    name.ends_with(".dex") && !name.contains('/')
}

pub fn project_path(root: &Path, target: &DecodeTarget) -> PathBuf {
    match target {
        DecodeTarget::Root(name) => root.join(name),
        DecodeTarget::Res(rel) => root.join("res").join(rel),
        DecodeTarget::Assets(rel) => root.join("assets").join(rel),
        DecodeTarget::Lib(rel) => root.join("lib").join(rel),
        DecodeTarget::Original(rel) => root.join("original").join(rel),
        DecodeTarget::Unknown(rel) => root.join("unknown").join(rel),
    }
}

/// Logical VFS path for a decode target under `root` (slash-separated).
pub fn project_path_vfs(root: &str, target: &DecodeTarget) -> String {
    use apk_patch_vfs::join_vfs;
    match target {
        DecodeTarget::Root(name) => join_vfs(root, name),
        DecodeTarget::Res(rel) => join_vfs(&join_vfs(root, "res"), rel),
        DecodeTarget::Assets(rel) => join_vfs(&join_vfs(root, "assets"), rel),
        DecodeTarget::Lib(rel) => join_vfs(&join_vfs(root, "lib"), rel),
        DecodeTarget::Original(rel) => join_vfs(&join_vfs(root, "original"), rel),
        DecodeTarget::Unknown(rel) => join_vfs(&join_vfs(root, "unknown"), rel),
    }
}

pub fn write_entry(root: &Path, target: &DecodeTarget, data: &[u8]) -> Result<()> {
    let path = project_path(root, target);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, data)?;
    Ok(())
}

pub fn write_entry_vfs(
    vfs: &mut dyn apk_patch_vfs::Vfs,
    root: &str,
    target: &DecodeTarget,
    data: &[u8],
) -> Result<()> {
    let path = project_path_vfs(root, target);
    if let Some(parent) = apk_patch_vfs::parent_vfs(&path) {
        vfs.create_dir_all(&parent)
            .map_err(|e| ProjectError::Project(e.to_string()))?;
    }
    vfs.write(&path, data)
        .map_err(|e| ProjectError::Project(e.to_string()))?;
    Ok(())
}

/// Collect APK entries from a decoded project for rebuild.
pub fn collect_build_entries(
    project: &Path,
    meta: &ApkToolMeta,
    assembled_dex: &std::collections::HashMap<String, Vec<u8>>,
) -> Result<Vec<BuildEntry>> {
    let mut entries = Vec::new();

    for target_dir in ["", "assets", "lib", "unknown", "res"] {
        let base = if target_dir.is_empty() {
            project.to_path_buf()
        } else {
            project.join(target_dir)
        };
        if !base.exists() {
            continue;
        }
        for entry in WalkDir::new(&base).into_iter().filter_map(|e| e.ok()) {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            let rel = path.strip_prefix(project).map_err(|e| {
                ProjectError::Project(format!("strip_prefix: {e}"))
            })?;
            let rel_str = rel.to_string_lossy().replace('\\', "/");

            if should_skip_on_build(&rel_str) {
                continue;
            }

            let apk_name = map_to_apk_path(&rel_str, target_dir);
            let data = std::fs::read(path)?;
            let compress = !should_store_uncompressed(&apk_name, &meta.doNotCompress);
            entries.push(BuildEntry {
                name: apk_name,
                data,
                compress,
            });
        }
    }

    // Root-level or original resources.arsc (decoded projects keep binary in original/)
    {
        let original_arsc = project.join("original/resources.arsc");
        let root_arsc = project.join("resources.arsc");
        let arsc_path = if original_arsc.is_file() {
            original_arsc
        } else {
            root_arsc
        };
        if arsc_path.is_file() {
            let data = std::fs::read(&arsc_path)?;
            let compress = !should_store_uncompressed("resources.arsc", &meta.doNotCompress);
            entries.push(BuildEntry {
                name: "resources.arsc".into(),
                data,
                compress,
            });
        }
    }

    for entry in std::fs::read_dir(project)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_dex_entry(&name) && entry.path().is_file() && !assembled_dex.contains_key(&name) {
            let data = std::fs::read(entry.path())?;
            let compress = !should_store_uncompressed(&name, &meta.doNotCompress);
            entries.push(BuildEntry {
                name,
                data,
                compress,
            });
        }
    }

    for (dex_name, data) in assembled_dex {
        let compress = !should_store_uncompressed(dex_name, &meta.doNotCompress);
        entries.push(BuildEntry {
            name: dex_name.clone(),
            data: data.clone(),
            compress,
        });
    }

    // Manifest: prefer edited text XML at project root; fall back to original binary.
    let text_manifest = project.join("AndroidManifest.xml");
    let original_manifest = project.join("original/AndroidManifest.xml");
    let manifest_path = if text_manifest.is_file() {
        text_manifest
    } else {
        original_manifest
    };
    if manifest_path.is_file() {
        let data = std::fs::read(&manifest_path)?;
        let compress = !should_store_uncompressed("AndroidManifest.xml", &meta.doNotCompress);
        entries.push(BuildEntry {
            name: "AndroidManifest.xml".into(),
            data,
            compress,
        });
    }

    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

/// Collect APK entries from a VFS-backed project tree.
pub fn collect_build_entries_vfs(
    vfs: &dyn apk_patch_vfs::Vfs,
    project: &str,
    meta: &ApkToolMeta,
    assembled_dex: &std::collections::HashMap<String, Vec<u8>>,
) -> Result<Vec<BuildEntry>> {
    use apk_patch_vfs::join_vfs;

    let mut entries = Vec::new();

    for target_dir in ["", "assets", "lib", "unknown", "res"] {
        let base = if target_dir.is_empty() {
            project.to_string()
        } else {
            join_vfs(project, target_dir)
        };
        if !vfs.exists(&base) {
            continue;
        }
        for path in vfs
            .walk_files(&base)
            .map_err(|e| ProjectError::Project(e.to_string()))?
        {
            let rel = path
                .strip_prefix(project)
                .map(|s| s.trim_start_matches('/'))
                .unwrap_or(path.as_str());
            if should_skip_on_build(rel) {
                continue;
            }
            let apk_name = map_to_apk_path(rel, target_dir);
            let data = vfs
                .read(&path)
                .map_err(|e| ProjectError::Project(e.to_string()))?;
            let compress = !should_store_uncompressed(&apk_name, &meta.doNotCompress);
            entries.push(BuildEntry {
                name: apk_name,
                data,
                compress,
            });
        }
    }

    {
        let original_arsc = join_vfs(project, "original/resources.arsc");
        let root_arsc = join_vfs(project, "resources.arsc");
        let arsc_path = if vfs.is_file(&original_arsc) {
            original_arsc
        } else {
            root_arsc
        };
        if vfs.is_file(&arsc_path) {
            let data = vfs
                .read(&arsc_path)
                .map_err(|e| ProjectError::Project(e.to_string()))?;
            let compress = !should_store_uncompressed("resources.arsc", &meta.doNotCompress);
            entries.push(BuildEntry {
                name: "resources.arsc".into(),
                data,
                compress,
            });
        }
    }

    if let Ok(rd) = vfs.read_dir(project) {
        for entry in rd {
            if is_dex_entry(&entry.name)
                && entry.is_file
                && !assembled_dex.contains_key(&entry.name)
            {
                let data = vfs
                    .read(&entry.path)
                    .map_err(|e| ProjectError::Project(e.to_string()))?;
                let compress = !should_store_uncompressed(&entry.name, &meta.doNotCompress);
                entries.push(BuildEntry {
                    name: entry.name,
                    data,
                    compress,
                });
            }
        }
    }

    for (dex_name, data) in assembled_dex {
        let compress = !should_store_uncompressed(dex_name, &meta.doNotCompress);
        entries.push(BuildEntry {
            name: dex_name.clone(),
            data: data.clone(),
            compress,
        });
    }

    let text_manifest = join_vfs(project, "AndroidManifest.xml");
    let original_manifest = join_vfs(project, "original/AndroidManifest.xml");
    let manifest_path = if vfs.is_file(&text_manifest) {
        text_manifest
    } else {
        original_manifest
    };
    if vfs.is_file(&manifest_path) {
        let data = vfs
            .read(&manifest_path)
            .map_err(|e| ProjectError::Project(e.to_string()))?;
        let compress = !should_store_uncompressed("AndroidManifest.xml", &meta.doNotCompress);
        entries.push(BuildEntry {
            name: "AndroidManifest.xml".into(),
            data,
            compress,
        });
    }

    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

#[derive(Debug, Clone)]
pub struct BuildEntry {
    pub name: String,
    pub data: Vec<u8>,
    pub compress: bool,
}

fn should_skip_on_build(rel: &str) -> bool {
    matches!(
        rel,
        "apktool.yml"
            | "original"
            | "dist"
            | "build"
            | "AndroidManifest.xml"
            | "resources.arsc"
    ) || rel.starts_with("original/")
        || rel.starts_with("dist/")
        || rel.starts_with("build/")
        || rel.starts_with("dex/")
        || rel.starts_with("dex_")
        // Decoded value XML is rebuilt from arsc later; don't pack as APK entries yet.
        || rel.starts_with("res/values")
}

fn map_to_apk_path(rel: &str, target_dir: &str) -> String {
    match target_dir {
        "assets" => {
            let stripped = rel.strip_prefix("assets/").unwrap_or(rel);
            format!("assets/{stripped}")
        }
        "lib" => {
            let stripped = rel.strip_prefix("lib/").unwrap_or(rel);
            format!("lib/{stripped}")
        }
        "res" => {
            let stripped = rel.strip_prefix("res/").unwrap_or(rel);
            format!("res/{stripped}")
        }
        "unknown" => rel.strip_prefix("unknown/").unwrap_or(rel).to_string(),
        _ => rel.to_string(),
    }
}

pub fn should_store_uncompressed(path: &str, do_not_compress: &[String]) -> bool {
    if do_not_compress.iter().any(|rule| path == rule) {
        return true;
    }
    if let Some(ext) = path.rsplit('.').next() {
        if do_not_compress.iter().any(|rule| rule == ext) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_entries() {
        assert_eq!(classify_entry("AndroidManifest.xml"), EntryKind::Manifest);
        assert_eq!(classify_entry("classes.dex"), EntryKind::Dex);
        assert_eq!(classify_entry("assets/foo.dat"), EntryKind::Asset);
        assert_eq!(classify_entry("META-INF/CERT.RSA"), EntryKind::Original);
    }
}
