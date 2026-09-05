//! `apkpatch.yml` project metadata for apk-patch.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum MetaError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("YAML error: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("{0}")]
    Missing(String),
}

pub type Result<T> = std::result::Result<T, MetaError>;

/// Canonical project metadata filename.
pub const META_FILENAME: &str = "apkpatch.yml";
/// Legacy Apktool-compatible filename still accepted on load.
pub const LEGACY_META_FILENAME: &str = "apktool.yml";
/// Directory holding preserved XAPK/APKM members (splits, manifest.json, icons).
pub const CONTAINER_DIR: &str = "container";

/// Resolve metadata path: prefer `apkpatch.yml`, fall back to legacy `apktool.yml`.
pub fn find_meta_path(project: &Path) -> Result<PathBuf> {
    let modern = project.join(META_FILENAME);
    if modern.is_file() {
        return Ok(modern);
    }
    let legacy = project.join(LEGACY_META_FILENAME);
    if legacy.is_file() {
        return Ok(legacy);
    }
    Err(MetaError::Missing(format!(
        "missing {META_FILENAME} (or legacy {LEGACY_META_FILENAME}) in {}",
        project.display()
    )))
}

/// VFS path for metadata (modern name first).
pub fn find_meta_path_vfs(is_file: impl Fn(&str) -> bool, project: &str) -> Result<String> {
    let modern = if project.is_empty() || project == "." {
        META_FILENAME.to_string()
    } else {
        format!(
            "{}/{}",
            project.trim_end_matches('/'),
            META_FILENAME
        )
    };
    if is_file(&modern) {
        return Ok(modern);
    }
    let legacy = if project.is_empty() || project == "." {
        LEGACY_META_FILENAME.to_string()
    } else {
        format!(
            "{}/{}",
            project.trim_end_matches('/'),
            LEGACY_META_FILENAME
        )
    };
    if is_file(&legacy) {
        return Ok(legacy);
    }
    Err(MetaError::Missing(format!(
        "missing {META_FILENAME} (or legacy {LEGACY_META_FILENAME}) in {project}"
    )))
}

/// True if `rel` is a metadata yaml that must not be packed into the APK.
pub fn is_meta_filename(name: &str) -> bool {
    name == META_FILENAME || name == LEGACY_META_FILENAME
}
/// Outer package type for decode/build.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PackageFormat {
    #[default]
    Apk,
    Xapk,
    Apkm,
}

impl PackageFormat {
    pub fn is_split_container(self) -> bool {
        matches!(self, Self::Xapk | Self::Apkm)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Apk => "apk",
            Self::Xapk => "xapk",
            Self::Apkm => "apkm",
        }
    }
}

/// Role of a member inside an XAPK/APKM container.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SplitMemberRole {
    Base,
    Split,
    Meta,
    Other,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SplitMember {
    pub name: String,
    pub role: SplitMemberRole,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SplitContainerMeta {
    /// Archive path of the editable base APK (e.g. `base.apk` or `com.foo.apk`).
    pub baseApk: String,
    pub members: Vec<SplitMember>,
}

/// Apktool 3.x project metadata.
#[allow(non_snake_case)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ApkToolMeta {
    pub version: String,
    pub apkFileName: String,
    /// `apk` (default), `xapk`, or `apkm`.
    #[serde(default)]
    pub packageFormat: PackageFormat,
    /// Present when [`Self::packageFormat`] is xapk/apkm.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub splitContainer: Option<SplitContainerMeta>,
    #[serde(default)]
    pub usesFramework: UsesFramework,
    #[serde(default)]
    pub usesLibrary: Vec<String>,
    #[serde(default)]
    pub sdkInfo: SdkInfo,
    #[serde(default)]
    pub versionInfo: VersionInfo,
    #[serde(default)]
    pub resourcesInfo: ResourcesInfo,
    #[serde(default)]
    pub featureFlags: serde_yaml::Value,
    #[serde(default)]
    pub doNotCompress: Vec<String>,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct UsesFramework {
    #[serde(default)]
    pub ids: Vec<u32>,
    #[serde(default)]
    pub tag: Option<String>,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SdkInfo {
    #[serde(default)]
    pub minSdkVersion: Option<String>,
    #[serde(default)]
    pub targetSdkVersion: Option<String>,
    #[serde(default)]
    pub maxSdkVersion: Option<String>,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct VersionInfo {
    #[serde(default)]
    pub versionCode: Option<u32>,
    #[serde(default)]
    pub versionName: Option<String>,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ResourcesInfo {
    #[serde(default)]
    pub packageId: Option<u32>,
    #[serde(default)]
    pub packageName: Option<String>,
    #[serde(default)]
    pub sparseEntries: bool,
    #[serde(default)]
    pub compactEntries: bool,
    #[serde(default)]
    pub keepRawValues: bool,
}

impl ApkToolMeta {
    pub fn new(apk_file_name: impl Into<String>) -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            apkFileName: apk_file_name.into(),
            packageFormat: PackageFormat::Apk,
            splitContainer: None,
            usesFramework: UsesFramework::default(),
            usesLibrary: Vec::new(),
            sdkInfo: SdkInfo::default(),
            versionInfo: VersionInfo::default(),
            resourcesInfo: ResourcesInfo::default(),
            featureFlags: serde_yaml::Value::Mapping(Default::default()),
            doNotCompress: default_do_not_compress(),
        }
    }

    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        Self::from_yaml(&raw)
    }

    /// Load from a project dir (`apkpatch.yml`, or legacy `apktool.yml`).
    pub fn load_from_project(project: &Path) -> Result<Self> {
        Self::load(&find_meta_path(project)?)
    }

    pub fn from_yaml(raw: &str) -> Result<Self> {
        let mut meta: Self = serde_yaml::from_str(raw)?;
        migrate_from_v2(&mut meta, raw);
        Ok(meta)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        std::fs::write(path, self.to_yaml()?)?;
        Ok(())
    }

    /// Write canonical `apkpatch.yml` into the project directory.
    pub fn save_to_project(&self, project: &Path) -> Result<()> {
        self.save(&project.join(META_FILENAME))
    }

    pub fn to_yaml(&self) -> Result<String> {
        Ok(serde_yaml::to_string(self)?)
    }
}

fn default_do_not_compress() -> Vec<String> {
    vec![
        "arsc".into(),
        "png".into(),
        "jpg".into(),
        "jpeg".into(),
        "gif".into(),
        "webp".into(),
        "wav".into(),
        "mp2".into(),
        "mp3".into(),
        "ogg".into(),
        "aac".into(),
        "mpg".into(),
        "mpeg".into(),
        "mid".into(),
        "midi".into(),
        "smf".into(),
        "jet".into(),
        "rtttl".into(),
        "imy".into(),
        "xmf".into(),
        "mp4".into(),
        "m4a".into(),
        "m4v".into(),
        "3gp".into(),
        "3gpp".into(),
        "3g2".into(),
        "3gpp2".into(),
        "amr".into(),
        "awb".into(),
        "wma".into(),
        "wmv".into(),
        "so".into(),
    ]
}

/// Migrate legacy 2.x fields when present in raw YAML.
fn migrate_from_v2(meta: &mut ApkToolMeta, raw: &str) {
    let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(raw) else {
        return;
    };
    let Some(mapping) = value.as_mapping() else {
        return;
    };
    if let Some(pkg) = mapping.get("packageInfo") {
        if let Some(forced) = pkg.get("forcedPackageId").and_then(|v| v.as_u64()) {
            meta.resourcesInfo.packageId = Some(forced as u32);
        }
        if let Some(name) = pkg.get("renameManifestPackage").and_then(|v| v.as_str()) {
            meta.resourcesInfo.packageName = Some(name.to_string());
        }
    }
    if let Some(sparse) = mapping.get("sparseResources").and_then(|v| v.as_bool()) {
        meta.resourcesInfo.sparseEntries = sparse;
    }
    if mapping.get("compactEntries").and_then(|v| v.as_bool()) == Some(true) {
        meta.resourcesInfo.compactEntries = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_meta_prefers_modern_over_legacy() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join(LEGACY_META_FILENAME);
        let modern = dir.path().join(META_FILENAME);
        let meta = ApkToolMeta::new("app.apk");
        meta.save(&legacy).unwrap();
        assert_eq!(find_meta_path(dir.path()).unwrap(), legacy);
        meta.save(&modern).unwrap();
        assert_eq!(find_meta_path(dir.path()).unwrap(), modern);
    }

    #[test]
    fn roundtrip_yaml() {
        let meta = ApkToolMeta::new("app.apk");
        let yaml = serde_yaml::to_string(&meta).unwrap();
        let loaded: ApkToolMeta = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(loaded.apkFileName, "app.apk");
        assert_eq!(loaded.packageFormat, PackageFormat::Apk);
    }

    #[test]
    fn roundtrip_xapk_meta() {
        let mut meta = ApkToolMeta::new("app.xapk");
        meta.packageFormat = PackageFormat::Xapk;
        meta.splitContainer = Some(SplitContainerMeta {
            baseApk: "com.example.apk".into(),
            members: vec![
                SplitMember {
                    name: "com.example.apk".into(),
                    role: SplitMemberRole::Base,
                },
                SplitMember {
                    name: "config.mdpi.apk".into(),
                    role: SplitMemberRole::Split,
                },
                SplitMember {
                    name: "manifest.json".into(),
                    role: SplitMemberRole::Meta,
                },
            ],
        });
        let yaml = meta.to_yaml().unwrap();
        let loaded = ApkToolMeta::from_yaml(&yaml).unwrap();
        assert_eq!(loaded.packageFormat, PackageFormat::Xapk);
        assert_eq!(
            loaded.splitContainer.as_ref().unwrap().baseApk,
            "com.example.apk"
        );
    }

    #[test]
    fn migrate_v2_package_info() {
        let raw = r#"
version: "2.9.0"
apkFileName: old.apk
packageInfo:
  forcedPackageId: 127
  renameManifestPackage: com.example.new
sparseResources: true
"#;
        let mut meta: ApkToolMeta = serde_yaml::from_str(raw).unwrap();
        migrate_from_v2(&mut meta, raw);
        assert_eq!(meta.resourcesInfo.packageId, Some(127));
        assert_eq!(
            meta.resourcesInfo.packageName.as_deref(),
            Some("com.example.new")
        );
        assert!(meta.resourcesInfo.sparseEntries);
    }
}
