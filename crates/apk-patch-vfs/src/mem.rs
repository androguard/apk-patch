//! In-memory VFS for browser / bytes APIs.

use std::collections::{BTreeMap, BTreeSet};

use crate::pathutil::{join_vfs, normalize_vfs_path, parent_vfs};
use crate::{DirEntry, Result, Vfs, VfsError};

/// In-memory project tree. File paths map to bytes; directories are tracked separately.
#[derive(Debug, Clone, Default)]
pub struct MemVfs {
    files: BTreeMap<String, Vec<u8>>,
    dirs: BTreeSet<String>,
}

impl MemVfs {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    pub fn files(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.files
    }

    pub fn into_files(self) -> BTreeMap<String, Vec<u8>> {
        self.files
    }

    pub fn from_files(files: BTreeMap<String, Vec<u8>>) -> Self {
        let mut vfs = Self::new();
        for (path, data) in files {
            let _ = vfs.write(&path, &data);
        }
        vfs
    }

    fn ensure_parents(&mut self, path: &str) {
        let mut cur = normalize_vfs_path(path);
        while let Some(parent) = parent_vfs(&cur) {
            self.dirs.insert(parent.clone());
            cur = parent;
        }
    }
}

impl Vfs for MemVfs {
    fn exists(&self, path: &str) -> bool {
        let n = normalize_vfs_path(path);
        self.files.contains_key(&n) || self.dirs.contains(&n) || n.is_empty()
    }

    fn is_file(&self, path: &str) -> bool {
        self.files.contains_key(&normalize_vfs_path(path))
    }

    fn is_dir(&self, path: &str) -> bool {
        let n = normalize_vfs_path(path);
        if n.is_empty() {
            return true;
        }
        if self.dirs.contains(&n) {
            return true;
        }
        // Implicit dir if any file lives under it.
        let prefix = format!("{n}/");
        self.files.keys().any(|k| k.starts_with(&prefix))
            || self.dirs.iter().any(|d| d.starts_with(&prefix))
    }

    fn read(&self, path: &str) -> Result<Vec<u8>> {
        let n = normalize_vfs_path(path);
        self.files
            .get(&n)
            .cloned()
            .ok_or_else(|| VfsError::NotFound(n))
    }

    fn write(&mut self, path: &str, data: &[u8]) -> Result<()> {
        let n = normalize_vfs_path(path);
        if n.is_empty() {
            return Err(VfsError::Vfs("cannot write empty path".into()));
        }
        self.ensure_parents(&n);
        self.files.insert(n, data.to_vec());
        Ok(())
    }

    fn create_dir_all(&mut self, path: &str) -> Result<()> {
        let n = normalize_vfs_path(path);
        if n.is_empty() {
            return Ok(());
        }
        self.ensure_parents(&n);
        self.dirs.insert(n);
        Ok(())
    }

    fn remove_file(&mut self, path: &str) -> Result<()> {
        let n = normalize_vfs_path(path);
        if self.files.remove(&n).is_none() {
            return Err(VfsError::NotFound(n));
        }
        Ok(())
    }

    fn remove_dir_all(&mut self, path: &str) -> Result<()> {
        let n = normalize_vfs_path(path);
        if n.is_empty() {
            self.files.clear();
            self.dirs.clear();
            return Ok(());
        }
        let prefix = format!("{n}/");
        self.files.retain(|k, _| !k.starts_with(&prefix) && k != &n);
        self.dirs
            .retain(|d| !d.starts_with(&prefix) && d != &n);
        Ok(())
    }

    fn read_dir(&self, path: &str) -> Result<Vec<DirEntry>> {
        let n = normalize_vfs_path(path);
        let prefix = if n.is_empty() {
            String::new()
        } else {
            format!("{n}/")
        };
        let mut names: BTreeSet<(String, bool)> = BTreeSet::new();
        for key in self.files.keys() {
            let rest = if prefix.is_empty() {
                key.as_str()
            } else if let Some(r) = key.strip_prefix(&prefix) {
                r
            } else {
                continue;
            };
            if let Some((name, _)) = rest.split_once('/') {
                names.insert((name.to_string(), true));
            } else if !rest.is_empty() {
                names.insert((rest.to_string(), false));
            }
        }
        for dir in &self.dirs {
            let rest = if prefix.is_empty() {
                dir.as_str()
            } else if let Some(r) = dir.strip_prefix(&prefix) {
                r
            } else {
                continue;
            };
            if let Some((name, _)) = rest.split_once('/') {
                names.insert((name.to_string(), true));
            } else if !rest.is_empty() {
                names.insert((rest.to_string(), true));
            }
        }
        Ok(names
            .into_iter()
            .map(|(name, is_dir)| {
                let path = if n.is_empty() {
                    name.clone()
                } else {
                    join_vfs(&n, &name)
                };
                let is_file = !is_dir && self.files.contains_key(&path);
                DirEntry {
                    name,
                    path: path.clone(),
                    is_dir: is_dir || self.is_dir(&path),
                    is_file,
                }
            })
            .collect())
    }

    fn walk_files(&self, path: &str) -> Result<Vec<String>> {
        let n = normalize_vfs_path(path);
        if n.is_empty() {
            return Ok(self.files.keys().cloned().collect());
        }
        if self.is_file(&n) {
            return Ok(vec![n]);
        }
        let prefix = format!("{n}/");
        Ok(self
            .files
            .keys()
            .filter(|k| k.starts_with(&prefix) || *k == &n)
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_read_walk() {
        let mut vfs = MemVfs::new();
        vfs.write("proj/a.txt", b"hi").unwrap();
        vfs.write("proj/res/b.xml", b"<x/>").unwrap();
        assert!(vfs.is_dir("proj"));
        assert!(vfs.is_file("proj/a.txt"));
        assert_eq!(vfs.read("proj/a.txt").unwrap(), b"hi");
        let files = vfs.walk_files("proj").unwrap();
        assert_eq!(files.len(), 2);
    }
}
