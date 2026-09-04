//! aapt2 compile / link wrapper (Apktool-style resource rebuild).

use std::path::{Path, PathBuf};
use std::process::Command;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum Aapt2Error {
    #[error("aapt2 not found (set --aapt or ANDROID_HOME / Android SDK build-tools)")]
    NotFound,
    #[error("aapt2 failed: {0}")]
    Failed(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Aapt2Error>;

/// Resolve an aapt2 binary path.
pub fn find_aapt2(custom: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = custom {
        if p.is_file() {
            return Some(p.to_path_buf());
        }
    }
    if let Ok(p) = which("aapt2") {
        return Some(p);
    }
    for root in sdk_roots() {
        let build_tools = root.join("build-tools");
        if let Ok(entries) = std::fs::read_dir(&build_tools) {
            let mut versions: Vec<PathBuf> = entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect();
            versions.sort();
            versions.reverse();
            for dir in versions {
                let candidate = dir.join("aapt2");
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

/// Resolve `android.jar` for a given API level (best effort).
pub fn find_android_jar(api: Option<u32>) -> Option<PathBuf> {
    for root in sdk_roots() {
        let platforms = root.join("platforms");
        if let Some(api) = api {
            let jar = platforms.join(format!("android-{api}")).join("android.jar");
            if jar.is_file() {
                return Some(jar);
            }
        }
        if let Ok(entries) = std::fs::read_dir(&platforms) {
            let mut dirs: Vec<PathBuf> = entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect();
            dirs.sort();
            dirs.reverse();
            for dir in dirs {
                let jar = dir.join("android.jar");
                if jar.is_file() {
                    return Some(jar);
                }
            }
        }
    }
    None
}

fn sdk_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for key in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Ok(v) = std::env::var(key) {
            roots.push(PathBuf::from(v));
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        roots.push(PathBuf::from(&home).join("Library/Android/sdk"));
        roots.push(PathBuf::from(&home).join("Android/Sdk"));
    }
    roots
}

fn which(bin: &str) -> std::io::Result<PathBuf> {
    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join(bin);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "not in PATH",
    ))
}

#[derive(Debug, Clone, Default)]
pub struct Aapt2CompileOptions {
    pub no_crunch: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Aapt2LinkOptions {
    /// Extra `-I` APKs / jars (frameworks, android.jar).
    pub include: Vec<PathBuf>,
    pub min_sdk: Option<String>,
    pub target_sdk: Option<String>,
    pub version_code: Option<u32>,
    pub version_name: Option<String>,
    pub package_id: Option<u32>,
    pub replace_version: bool,
    /// Prefer directory output (`--output-to-dir`) when true.
    pub output_to_dir: bool,
}

/// `aapt2 compile --dir res -o resources.zip [--no-crunch]`
pub fn aapt2_compile(
    aapt2: &Path,
    res_dir: &Path,
    out_zip: &Path,
    options: &Aapt2CompileOptions,
) -> Result<()> {
    if let Some(parent) = out_zip.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut cmd = Command::new(aapt2);
    cmd.arg("compile").arg("--dir").arg(res_dir).arg("-o").arg(out_zip);
    if options.no_crunch {
        cmd.arg("--no-crunch");
    }
    run_aapt2(&mut cmd)
}

/// `aapt2 link` compiled resources + manifest → APK or directory.
pub fn aapt2_link(
    aapt2: &Path,
    compiled: &Path,
    manifest: &Path,
    output: &Path,
    options: &Aapt2LinkOptions,
) -> Result<()> {
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if options.output_to_dir {
        std::fs::create_dir_all(output)?;
    }
    let mut cmd = Command::new(aapt2);
    cmd.arg("link")
        .arg("-o")
        .arg(output)
        .arg("--manifest")
        .arg(manifest)
        .arg(compiled);
    if options.output_to_dir {
        cmd.arg("--output-to-dir");
    }
    for inc in &options.include {
        cmd.arg("-I").arg(inc);
    }
    if let Some(v) = &options.min_sdk {
        cmd.arg("--min-sdk-version").arg(v);
    }
    if let Some(v) = &options.target_sdk {
        cmd.arg("--target-sdk-version").arg(v);
    }
    if let Some(v) = options.version_code {
        cmd.arg("--version-code").arg(v.to_string());
    }
    if let Some(v) = &options.version_name {
        cmd.arg("--version-name").arg(v);
    }
    if options.replace_version {
        cmd.arg("--replace-version");
    }
    if let Some(id) = options.package_id {
        if id >= 0x7f {
            cmd.arg("--package-id").arg(format!("0x{id:x}"));
        }
    }
    run_aapt2(&mut cmd)
}

fn run_aapt2(cmd: &mut Command) -> Result<()> {
    let output = cmd.output()?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    Err(Aapt2Error::Failed(format!(
        "{}\n{}",
        stderr.trim(),
        stdout.trim()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_aapt2_prefers_existing_custom() {
        // When custom path is missing, fall back to PATH / SDK (may or may not exist).
        let _ = find_aapt2(Some(Path::new("/no/such/aapt2")));
        // If SDK aapt2 exists, find_aapt2(None) should locate it.
        if let Some(p) = find_aapt2(None) {
            assert!(p.is_file(), "{}", p.display());
            assert!(find_aapt2(Some(&p)).as_ref() == Some(&p));
        }
    }
}
