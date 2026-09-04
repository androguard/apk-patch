//! Virtual filesystem for apk-patch decode/build.
//!
//! Native CLI uses [`StdFs`] (real disk). Browser / bytes APIs use [`MemVfs`].

mod mem;
mod pathutil;

#[cfg(feature = "native-fs")]
mod stdfs;

pub use mem::MemVfs;
pub use pathutil::{join_vfs, normalize_vfs_path, parent_vfs, VfsPath};

#[cfg(feature = "native-fs")]
pub use stdfs::StdFs;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum VfsError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("vfs error: {0}")]
    Vfs(String),
    #[error("not found: {0}")]
    NotFound(String),
}

pub type Result<T> = std::result::Result<T, VfsError>;

/// Directory entry from [`Vfs::read_dir`].
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_file: bool,
}

/// Virtual filesystem used by decode/build.
pub trait Vfs: Send + Sync {
    fn exists(&self, path: &str) -> bool;
    fn is_file(&self, path: &str) -> bool;
    fn is_dir(&self, path: &str) -> bool;

    fn read(&self, path: &str) -> Result<Vec<u8>>;
    fn read_to_string(&self, path: &str) -> Result<String> {
        let bytes = self.read(path)?;
        String::from_utf8(bytes).map_err(|e| VfsError::Vfs(format!("utf8: {e}")))
    }
    fn write(&mut self, path: &str, data: &[u8]) -> Result<()>;
    fn create_dir_all(&mut self, path: &str) -> Result<()>;
    fn remove_file(&mut self, path: &str) -> Result<()>;
    fn remove_dir_all(&mut self, path: &str) -> Result<()>;
    fn copy(&mut self, from: &str, to: &str) -> Result<()> {
        let data = self.read(from)?;
        if let Some(parent) = parent_vfs(to) {
            self.create_dir_all(&parent)?;
        }
        self.write(to, &data)
    }

    /// Immediate children of `path` (non-recursive).
    fn read_dir(&self, path: &str) -> Result<Vec<DirEntry>>;

    /// Recursive file listing under `path` (inclusive of nested files).
    fn walk_files(&self, path: &str) -> Result<Vec<String>>;
}

/// Snapshot a VFS subtree into an owned map (logical path → bytes).
pub fn export_tree(vfs: &dyn Vfs, root: &str) -> Result<std::collections::BTreeMap<String, Vec<u8>>> {
    let mut out = std::collections::BTreeMap::new();
    let root_n = normalize_vfs_path(root);
    for path in vfs.walk_files(&root_n)? {
        let data = vfs.read(&path)?;
        out.insert(path, data);
    }
    Ok(out)
}

/// Import a flat map into `vfs` (overwrites existing files).
pub fn import_tree(
    vfs: &mut dyn Vfs,
    files: &std::collections::BTreeMap<String, Vec<u8>>,
) -> Result<()> {
    for (path, data) in files {
        if let Some(parent) = parent_vfs(path) {
            vfs.create_dir_all(&parent)?;
        }
        vfs.write(path, data)?;
    }
    Ok(())
}
