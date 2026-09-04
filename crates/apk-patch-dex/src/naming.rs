//! DEX ↔ project directory naming.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct DexDirEntry {
    pub dir_name: String,
    pub apk_dex_name: String,
}

pub fn dex_dir_name(apk_dex_name: &str) -> String {
    if apk_dex_name == "classes.dex" {
        return "dex".into();
    }
    if apk_dex_name.starts_with("classes") && apk_dex_name.ends_with(".dex") && !apk_dex_name.contains('/') {
        let stem = apk_dex_name.trim_end_matches(".dex");
        return format!("dex_{stem}");
    }
    format!("dex_{}", apk_dex_name.replace('/', "@"))
}

pub fn dex_apk_name(dir_name: &str) -> Option<String> {
    if dir_name == "dex" {
        return Some("classes.dex".into());
    }
    if let Some(stem) = dir_name.strip_prefix("dex_") {
        if stem.starts_with("classes") {
            return Some(format!("{stem}.dex"));
        }
        return Some(stem.replace('@', "/"));
    }
    None
}

pub fn is_odex(name: &str) -> bool {
    name.ends_with(".odex") || name.ends_with(".dey")
}

pub fn list_dex_dirs(project: &Path) -> std::io::Result<Vec<DexDirEntry>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(project)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "dex" || name.starts_with("dex_") {
            if let Some(apk_dex_name) = dex_apk_name(&name) {
                out.push(DexDirEntry {
                    dir_name: name,
                    apk_dex_name,
                });
            }
        }
    }
    out.sort_by(|a, b| a.apk_dex_name.cmp(&b.apk_dex_name));
    Ok(out)
}

pub fn list_dex_dirs_vfs(
    vfs: &dyn apk_patch_vfs::Vfs,
    project: &str,
) -> Result<Vec<DexDirEntry>, std::io::Error> {
    let mut out = Vec::new();
    let entries = vfs.read_dir(project).map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::Other, e.to_string())
    })?;
    for entry in entries {
        if !entry.is_dir {
            continue;
        }
        let name = entry.name;
        if name == "dex" || name.starts_with("dex_") {
            if let Some(apk_dex_name) = dex_apk_name(&name) {
                out.push(DexDirEntry {
                    dir_name: name,
                    apk_dex_name,
                });
            }
        }
    }
    out.sort_by(|a, b| a.apk_dex_name.cmp(&b.apk_dex_name));
    Ok(out)
}

pub fn class_descriptor_to_path(descriptor: &str) -> PathBuf {
    let inner = descriptor
        .strip_prefix('L')
        .and_then(|s| s.strip_suffix(';'))
        .unwrap_or(descriptor);
    let mut path = PathBuf::from(inner);
    path.set_extension("dex.txt");
    path
}

pub fn path_to_class_descriptor(path: &Path) -> String {
    // Foo.dex.txt → Foo (extension is the final ".txt")
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let stem = stem.strip_suffix(".dex").unwrap_or(stem);
    let parent = path
        .parent()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .filter(|p| !p.is_empty() && p != ".")
        .unwrap_or_default();
    let rel = if parent.is_empty() {
        stem.to_string()
    } else {
        format!("{parent}/{stem}")
    };
    format!("L{rel};")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn naming_roundtrip() {
        assert_eq!(dex_dir_name("classes.dex"), "dex");
        assert_eq!(dex_dir_name("classes2.dex"), "dex_classes2");
        assert_eq!(dex_apk_name("dex_classes2"), Some("classes2.dex".into()));
    }

    #[test]
    fn class_path_preserves_leading_l_in_name() {
        assert_eq!(
            class_descriptor_to_path("LLoop;").to_string_lossy(),
            "Loop.dex.txt"
        );
        assert_eq!(
            class_descriptor_to_path("Lcom/example/Foo;").to_string_lossy(),
            "com/example/Foo.dex.txt"
        );
        assert_eq!(
            path_to_class_descriptor(Path::new("Loop.dex.txt")),
            "LLoop;"
        );
        assert_eq!(
            path_to_class_descriptor(Path::new("com/example/Foo.dex.txt")),
            "Lcom/example/Foo;"
        );
    }
}
