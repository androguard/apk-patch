//! Real filesystem backend (native CLI).

use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::pathutil::normalize_vfs_path;
use crate::{DirEntry, Result, Vfs, VfsError};

/// Interprets VFS paths as OS paths (slash-normalized, then `PathBuf`).
#[derive(Debug, Clone, Default)]
pub struct StdFs;

impl StdFs {
    pub fn new() -> Self {
        Self
    }

    fn to_os(path: &str) -> PathBuf {
        let n = normalize_vfs_path(path);
        if n.is_empty() {
            PathBuf::from(".")
        } else {
            // Path::new accepts `/` on all platforms Rust supports.
            PathBuf::from(&n)
        }
    }

    fn from_os(path: &Path) -> String {
        normalize_vfs_path(&path.to_string_lossy())
    }
}

impl Vfs for StdFs {
    fn exists(&self, path: &str) -> bool {
        Self::to_os(path).exists()
    }

    fn is_file(&self, path: &str) -> bool {
        Self::to_os(path).is_file()
    }

    fn is_dir(&self, path: &str) -> bool {
        Self::to_os(path).is_dir()
    }

    fn read(&self, path: &str) -> Result<Vec<u8>> {
        let p = Self::to_os(path);
        std::fs::read(&p).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                VfsError::NotFound(path.to_string())
            } else {
                VfsError::Io(e)
            }
        })
    }

    fn write(&mut self, path: &str, data: &[u8]) -> Result<()> {
        let p = Self::to_os(path);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(p, data)?;
        Ok(())
    }

    fn create_dir_all(&mut self, path: &str) -> Result<()> {
        std::fs::create_dir_all(Self::to_os(path))?;
        Ok(())
    }

    fn remove_file(&mut self, path: &str) -> Result<()> {
        std::fs::remove_file(Self::to_os(path))?;
        Ok(())
    }

    fn remove_dir_all(&mut self, path: &str) -> Result<()> {
        let p = Self::to_os(path);
        if p.exists() {
            std::fs::remove_dir_all(p)?;
        }
        Ok(())
    }

    fn read_dir(&self, path: &str) -> Result<Vec<DirEntry>> {
        let p = Self::to_os(path);
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&p)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let full = entry.path();
            out.push(DirEntry {
                name,
                path: Self::from_os(&full),
                is_dir: full.is_dir(),
                is_file: full.is_file(),
            });
        }
        Ok(out)
    }

    fn walk_files(&self, path: &str) -> Result<Vec<String>> {
        let p = Self::to_os(path);
        if p.is_file() {
            return Ok(vec![Self::from_os(&p)]);
        }
        let mut out = Vec::new();
        for entry in WalkDir::new(&p).into_iter().filter_map(|e| e.ok()) {
            if entry.file_type().is_file() {
                out.push(Self::from_os(entry.path()));
            }
        }
        Ok(out)
    }
}
