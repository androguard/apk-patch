//! Slash-normalized virtual paths (always `/`, no leading `/` for relative roots).

/// Opaque helper marker for documentation.
pub type VfsPath = String;

pub fn normalize_vfs_path(path: &str) -> String {
    let mut out = String::new();
    for part in path.replace('\\', "/").split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            if let Some(idx) = out.rfind('/') {
                out.truncate(idx);
            } else {
                out.clear();
            }
            continue;
        }
        if !out.is_empty() {
            out.push('/');
        }
        out.push_str(part);
    }
    out
}

pub fn join_vfs(base: &str, rel: &str) -> String {
    let base = normalize_vfs_path(base);
    let rel = normalize_vfs_path(rel);
    if base.is_empty() {
        rel
    } else if rel.is_empty() {
        base
    } else {
        format!("{base}/{rel}")
    }
}

pub fn parent_vfs(path: &str) -> Option<String> {
    let n = normalize_vfs_path(path);
    n.rfind('/').map(|i| n[..i].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_and_join() {
        assert_eq!(normalize_vfs_path("/a//b/../c"), "a/c");
        assert_eq!(join_vfs("proj", "res/values"), "proj/res/values");
        assert_eq!(parent_vfs("a/b/c.txt").as_deref(), Some("a/b"));
    }
}
