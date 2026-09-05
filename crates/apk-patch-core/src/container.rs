//! XAPK / APKM split-container decode and rebuild.

use std::io::{Cursor, Write};
use std::path::Path;

use apk_patch_meta::{
    ApkToolMeta, PackageFormat, SplitContainerMeta, SplitMember, SplitMemberRole, CONTAINER_DIR,
};
use apkparser::{looks_like_apkm, ApkmArchive, ApkmEntryKind, ZipEntry};
use log::info;
use serde_json::Value as JsonValue;

use crate::build::{BuildError, Result as BuildResult};
use crate::decode::{DecodeError, Result as DecodeResult};

/// Detected split package (XAPK or APKM-shaped ZIP).
pub struct SplitPackage {
    pub format: PackageFormat,
    pub base_apk: String,
    pub members: Vec<SplitMember>,
    zip: ZipEntry,
}

impl SplitPackage {
    pub fn open(data: &[u8]) -> DecodeResult<Option<Self>> {
        if looks_like_xapk(data) {
            return Ok(Some(Self::from_xapk(data)?));
        }
        if looks_like_apkm(data) {
            return Ok(Some(Self::from_apkm(data)?));
        }
        Ok(None)
    }

    fn from_xapk(data: &[u8]) -> DecodeResult<Self> {
        let zip = ZipEntry::parse(data).map_err(DecodeError::Apk)?;
        let manifest = read_utf8_entry(&zip, "manifest.json").ok_or_else(|| {
            DecodeError::Decode("XAPK missing manifest.json".into())
        })?;
        let base_apk = base_from_xapk_manifest(&manifest)
            .or_else(|| prefer_base_apk_name(zip.namelist()))
            .ok_or_else(|| DecodeError::Decode("XAPK has no base APK".into()))?;
        let members = classify_zip_members(&zip, &base_apk, PackageFormat::Xapk);
        Ok(Self {
            format: PackageFormat::Xapk,
            base_apk,
            members,
            zip,
        })
    }

    fn from_apkm(data: &[u8]) -> DecodeResult<Self> {
        let arch = ApkmArchive::from_bytes(data).map_err(DecodeError::Apk)?;
        let base_apk = arch.base_name().to_string();
        let zip = ZipEntry::parse(data).map_err(DecodeError::Apk)?;
        let members = arch
            .entries()
            .iter()
            .map(|e| SplitMember {
                name: e.name.clone(),
                role: match e.kind {
                    ApkmEntryKind::Base => SplitMemberRole::Base,
                    ApkmEntryKind::Split => SplitMemberRole::Split,
                    ApkmEntryKind::Meta => SplitMemberRole::Meta,
                    ApkmEntryKind::Other => SplitMemberRole::Other,
                },
            })
            .collect();
        Ok(Self {
            format: PackageFormat::Apkm,
            base_apk,
            members,
            zip,
        })
    }

    pub fn base_apk_bytes(&self) -> DecodeResult<Vec<u8>> {
        self.zip
            .read_to_vec(&self.base_apk)
            .map_err(DecodeError::Apk)
    }

    pub fn to_meta(&self) -> SplitContainerMeta {
        SplitContainerMeta {
            baseApk: self.base_apk.clone(),
            members: self.members.clone(),
        }
    }

    /// Write non-base members under `project/container/` (preserve original names).
    pub fn write_preserved_members(&self, project: &Path) -> DecodeResult<()> {
        let root = project.join(CONTAINER_DIR);
        std::fs::create_dir_all(&root)?;
        for m in &self.members {
            if m.role == SplitMemberRole::Base {
                continue;
            }
            let bytes = self.zip.read_to_vec(&m.name).map_err(DecodeError::Apk)?;
            let dest = root.join(safe_container_path(&m.name));
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&dest, bytes)?;
        }
        // Also keep a copy of the original base for reference / fallback.
        let base_bytes = self.base_apk_bytes()?;
        let base_dest = root.join("original_base").join(safe_container_path(&self.base_apk));
        if let Some(parent) = base_dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&base_dest, base_bytes)?;
        Ok(())
    }
}

/// True when ZIP looks like an APKPure-style XAPK (`manifest.json` + split APKs).
pub fn looks_like_xapk(raw: &[u8]) -> bool {
    if raw.len() < 4 || !raw.starts_with(b"PK") {
        return false;
    }
    let Ok(zip) = ZipEntry::parse(raw) else {
        return false;
    };
    if zip_has_root_manifest(&zip) {
        return false;
    }
    let Some(manifest) = read_utf8_entry(&zip, "manifest.json") else {
        return false;
    };
    if let Ok(v) = serde_json::from_str::<JsonValue>(&manifest) {
        if v.get("xapk_version").is_some() || v.get("split_apks").is_some() {
            return prefer_base_apk_name(zip.namelist()).is_some()
                || base_from_xapk_manifest(&manifest).is_some();
        }
    }
    false
}

fn base_from_xapk_manifest(manifest: &str) -> Option<String> {
    let v: JsonValue = serde_json::from_str(manifest).ok()?;
    let splits = v.get("split_apks")?.as_array()?;
    for entry in splits {
        let id = entry.get("id").and_then(|x| x.as_str()).unwrap_or("");
        let file = entry.get("file").and_then(|x| x.as_str())?;
        if id.eq_ignore_ascii_case("base") {
            return Some(file.to_string());
        }
    }
    splits
        .first()
        .and_then(|e| e.get("file"))
        .and_then(|x| x.as_str())
        .map(str::to_string)
}

fn prefer_base_apk_name(names: &[String]) -> Option<String> {
    if let Some(n) = names.iter().find(|n| {
        let file = n.rsplit('/').next().unwrap_or(n);
        file.eq_ignore_ascii_case("base.apk")
    }) {
        return Some(n.clone());
    }
    let apks: Vec<&String> = names.iter().filter(|n| is_apk_entry(n)).collect();
    if apks.len() == 1 {
        return Some(apks[0].clone());
    }
    apks.into_iter()
        .find(|n| {
            let file = n.rsplit('/').next().unwrap_or(n).to_ascii_lowercase();
            !file.starts_with("split_config.") && !file.starts_with("config.")
        })
        .cloned()
}

fn classify_zip_members(zip: &ZipEntry, base_apk: &str, format: PackageFormat) -> Vec<SplitMember> {
    zip.namelist()
        .iter()
        .map(|name| {
            let lower = name.to_ascii_lowercase();
            let role = if name == base_apk {
                SplitMemberRole::Base
            } else if is_apk_entry(name) {
                SplitMemberRole::Split
            } else if lower == "manifest.json"
                || lower == "info.json"
                || lower.ends_with("icon.png")
                || lower.contains("apkm_installer")
                || lower.starts_with("meta-inf/")
            {
                SplitMemberRole::Meta
            } else if format == PackageFormat::Xapk && lower.ends_with(".json") {
                SplitMemberRole::Meta
            } else {
                SplitMemberRole::Other
            };
            SplitMember {
                name: name.clone(),
                role,
            }
        })
        .collect()
}

fn is_apk_entry(name: &str) -> bool {
    let file = name.rsplit('/').next().unwrap_or(name);
    file.to_ascii_lowercase().ends_with(".apk") && !file.starts_with('.')
}

fn zip_has_root_manifest(zip: &ZipEntry) -> bool {
    zip.namelist().iter().any(|n| n == "AndroidManifest.xml") || zip.contains("AndroidManifest.xml")
}

fn read_utf8_entry(zip: &ZipEntry, name: &str) -> Option<String> {
    let bytes = zip.read_to_vec(name).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn safe_container_path(name: &str) -> String {
    name.trim_start_matches('/').replace('\\', "/")
}

/// Patch `apkpatch.yml` after a base-APK decode to record the outer container.
pub fn stamp_container_meta(
    project: &Path,
    package_name: &str,
    format: PackageFormat,
    split: SplitContainerMeta,
) -> DecodeResult<()> {
    let mut meta = ApkToolMeta::load_from_project(project)?;
    meta.apkFileName = package_name.to_string();
    meta.packageFormat = format;
    meta.splitContainer = Some(split);
    meta.save_to_project(project)?;
    Ok(())
}

/// Rebuild outer XAPK/APKM: replace base APK bytes, copy preserved members.
pub fn pack_split_container(
    project: &Path,
    meta: &ApkToolMeta,
    rebuilt_base_apk: &[u8],
) -> BuildResult<Vec<u8>> {
    let split = meta.splitContainer.as_ref().ok_or_else(|| {
        BuildError::Build("packageFormat is xapk/apkm but splitContainer meta is missing".into())
    })?;
    let container_root = project.join(CONTAINER_DIR);
    if !container_root.is_dir() {
        return Err(BuildError::Build(format!(
            "missing {CONTAINER_DIR}/ (required to rebuild {})",
            meta.packageFormat.as_str()
        )));
    }

    info!(
        "I: packing {} ({} members, base={})",
        meta.packageFormat.as_str(),
        split.members.len(),
        split.baseApk
    );

    let cursor = Cursor::new(Vec::new());
    let mut zip = zip::ZipWriter::new(cursor);
    let opts =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);

    for m in &split.members {
        zip.start_file(m.name.as_str(), opts)
            .map_err(|e| BuildError::Build(e.to_string()))?;
        if m.role == SplitMemberRole::Base || m.name == split.baseApk {
            zip.write_all(rebuilt_base_apk)
                .map_err(|e| BuildError::Build(e.to_string()))?;
            continue;
        }
        let path = container_root.join(safe_container_path(&m.name));
        if !path.is_file() {
            return Err(BuildError::Build(format!(
                "missing container member {}",
                path.display()
            )));
        }
        let bytes = std::fs::read(&path)?;
        zip.write_all(&bytes)
            .map_err(|e| BuildError::Build(e.to_string()))?;
    }

    let cursor = zip
        .finish()
        .map_err(|e| BuildError::Build(e.to_string()))?;
    Ok(cursor.into_inner())
}

/// Strip known Android package extensions for default output dir naming.
pub fn strip_package_extension(name: &str) -> &str {
    const EXTS: &[&str] = &[".xapk", ".apkm", ".apk", ".apks"];
    for ext in EXTS {
        if let Some(stem) = name
            .strip_suffix(ext)
            .or_else(|| name.strip_suffix(&ext.to_ascii_uppercase()))
        {
            if !stem.is_empty() {
                return stem;
            }
        }
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn minimal_apk(label: &[u8]) -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(cursor);
        let opts =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
        zip.start_file("AndroidManifest.xml", opts).unwrap();
        zip.write_all(label).unwrap();
        zip.start_file("classes.dex", opts).unwrap();
        zip.write_all(b"dex\n000").unwrap();
        zip.finish().unwrap().into_inner()
    }

    fn write_test_xapk(base: &[u8], split: &[u8]) -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(cursor);
        let opts =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
        let manifest = r#"{
  "xapk_version": 2,
  "package_name": "com.example.test",
  "split_apks": [
    {"file": "com.example.test.apk", "id": "base"},
    {"file": "config.mdpi.apk", "id": "config.mdpi"}
  ]
}"#;
        zip.start_file("manifest.json", opts).unwrap();
        zip.write_all(manifest.as_bytes()).unwrap();
        zip.start_file("com.example.test.apk", opts).unwrap();
        zip.write_all(base).unwrap();
        zip.start_file("config.mdpi.apk", opts).unwrap();
        zip.write_all(split).unwrap();
        zip.start_file("icon.png", opts).unwrap();
        zip.write_all(b"PNG").unwrap();
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn detect_xapk_and_base() {
        let base = minimal_apk(b"BASE");
        let split = minimal_apk(b"SPLIT");
        let xapk = write_test_xapk(&base, &split);
        assert!(looks_like_xapk(&xapk));
        let pkg = SplitPackage::open(&xapk).unwrap().unwrap();
        assert_eq!(pkg.format, PackageFormat::Xapk);
        assert_eq!(pkg.base_apk, "com.example.test.apk");
        assert_eq!(pkg.base_apk_bytes().unwrap(), base);
    }

    #[test]
    fn pack_replaces_base_only() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path();
        let base = minimal_apk(b"BASE");
        let split = minimal_apk(b"SPLIT");
        let xapk = write_test_xapk(&base, &split);
        let pkg = SplitPackage::open(&xapk).unwrap().unwrap();
        pkg.write_preserved_members(project).unwrap();

        let mut meta = ApkToolMeta::new("app.xapk");
        meta.packageFormat = PackageFormat::Xapk;
        meta.splitContainer = Some(pkg.to_meta());

        let rebuilt = minimal_apk(b"REBUILT");
        let out = pack_split_container(project, &meta, &rebuilt).unwrap();
        let zip = ZipEntry::parse(&out).unwrap();
        assert_eq!(zip.read_to_vec("com.example.test.apk").unwrap(), rebuilt);
        assert_eq!(zip.read_to_vec("config.mdpi.apk").unwrap(), split);
        let man = String::from_utf8(zip.read_to_vec("manifest.json").unwrap()).unwrap();
        assert!(man.contains("xapk_version"));
    }
}
