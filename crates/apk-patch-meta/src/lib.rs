//! `apktool.yml` metadata for apk-patch projects.

use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum MetaError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("YAML error: {0}")]
    Yaml(#[from] serde_yaml::Error),
}

pub type Result<T> = std::result::Result<T, MetaError>;

pub const META_FILENAME: &str = "apktool.yml";

/// Apktool 3.x project metadata.
#[allow(non_snake_case)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ApkToolMeta {
    pub version: String,
    pub apkFileName: String,
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
        let mut meta: Self = serde_yaml::from_str(&raw)?;
        migrate_from_v2(&mut meta, &raw);
        Ok(meta)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let yaml = serde_yaml::to_string(self)?;
        std::fs::write(path, yaml)?;
        Ok(())
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
    fn roundtrip_yaml() {
        let meta = ApkToolMeta::new("app.apk");
        let yaml = serde_yaml::to_string(&meta).unwrap();
        let loaded: ApkToolMeta = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(loaded.apkFileName, "app.apk");
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
