//! One-shot inject of `libgoauld_agent.so` + early-load ContentProvider.

use std::path::{Path, PathBuf};

use apk_patch_meta::{ApkToolMeta, META_FILENAME};
use apk_patch_sign::BuildSignConfig;
use log::info;
use thiserror::Error;

use crate::build::{build_project, BuildOptions};
use crate::decode::{decode_apk, DecodeOptions};
use crate::manifest_patch::insert_goauld_loader_provider;

/// Embedded DEX containing `goauld.inject.LoaderProvider`.
pub const GOAULD_LOADER_DEX: &[u8] = include_bytes!("../assets/goauld_loader.dex");

const AGENT_SO_NAME: &str = "libgoauld_agent.so";
const AGENT_ABI_DIR: &str = "arm64-v8a";
const DEFAULT_AGENT_REL: &str = "../arm_goauld/dist/android-arm64/libgoauld_agent.so";
const ENV_AGENT: &str = "GOAULD_AGENT_SO";

#[derive(Error, Debug)]
pub enum InjectError {
    #[error(transparent)]
    Decode(#[from] crate::decode::DecodeError),
    #[error(transparent)]
    Build(#[from] crate::build::BuildError),
    #[error(transparent)]
    Meta(#[from] apk_patch_meta::MetaError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("inject error: {0}")]
    Inject(String),
}

pub type Result<T> = std::result::Result<T, InjectError>;

#[derive(Debug, Clone)]
pub struct InjectGoauldOptions {
    /// Path to `libgoauld_agent.so` (android-arm64).
    pub agent_so: PathBuf,
    /// Final APK path (default: beside input as `*-goauld.apk`).
    pub output: Option<PathBuf>,
    pub force: bool,
    /// Keep / use this project directory instead of a temp dir.
    pub work_dir: Option<PathBuf>,
    pub sign: BuildSignConfig,
    pub jobs: usize,
}

impl Default for InjectGoauldOptions {
    fn default() -> Self {
        Self {
            agent_so: default_agent_so_path(),
            output: None,
            force: false,
            work_dir: None,
            sign: BuildSignConfig::default(),
            jobs: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
                .min(8),
        }
    }
}

/// Default agent path: `$GOAULD_AGENT_SO`, else `../arm_goauld/dist/android-arm64/libgoauld_agent.so`.
pub fn default_agent_so_path() -> PathBuf {
    if let Ok(p) = std::env::var(ENV_AGENT) {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    PathBuf::from(DEFAULT_AGENT_REL)
}

fn log_agent_so(agent_so: &Path) {
    let meta = std::fs::metadata(agent_so).ok();
    let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let canonical = std::fs::canonicalize(agent_so)
        .unwrap_or_else(|_| agent_so.to_path_buf());
    let file_name = agent_so
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(AGENT_SO_NAME);
    let source = agent_so_source(agent_so);
    info!("I: goauld agent .so");
    info!("I:   file     = {file_name}");
    info!("I:   path     = {}", agent_so.display());
    if canonical != agent_so {
        info!("I:   resolved = {}", canonical.display());
    }
    info!("I:   size     = {size} bytes");
    info!("I:   source   = {source}");
    info!("I:   apk path = lib/{AGENT_ABI_DIR}/{AGENT_SO_NAME}");
    info!("I:   load via = System.loadLibrary(\"goauld_agent\")");
}

fn agent_so_source(agent_so: &Path) -> String {
    if let Ok(env) = std::env::var(ENV_AGENT) {
        if !env.is_empty() && Path::new(&env) == agent_so {
            return format!("${ENV_AGENT}");
        }
    }
    if agent_so == Path::new(DEFAULT_AGENT_REL) {
        return format!("default ({DEFAULT_AGENT_REL})");
    }
    "--agent".into()
}

/// Decode → inject SO + loader DEX + provider → build → signed APK.
pub fn inject_goauld(apk: &Path, opts: &InjectGoauldOptions) -> Result<PathBuf> {
    if !opts.agent_so.is_file() {
        return Err(InjectError::Inject(format!(
            "goauld agent not found: {} (set --agent or {ENV_AGENT})",
            opts.agent_so.display()
        )));
    }

    log_agent_so(&opts.agent_so);

    let keep_project = opts.work_dir.is_some();
    let project_dir = if let Some(ref dir) = opts.work_dir {
        dir.clone()
    } else {
        let tmp = tempfile_dir()?;
        tmp
    };

    let decode_opts = DecodeOptions {
        force: opts.force || !keep_project,
        no_src: true,
        no_res: false,
        no_assets: false,
        all_src: true,
        output: Some(project_dir.clone()),
        jobs: opts.jobs,
        ..DecodeOptions::default()
    };
    let decoded = decode_apk(apk, &decode_opts)?;
    let project = decoded.output_dir;
    info!("I: decoded to {}", project.display());

    apply_goauld_inject(&project, &opts.agent_so)?;

    let out_apk = opts.output.clone().unwrap_or_else(|| {
        let stem = apk
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("app");
        apk.parent()
            .unwrap_or_else(|| Path::new("."))
            .join(format!("{stem}-goauld.apk"))
    });

    let build_opts = BuildOptions {
        force: true,
        output: Some(out_apk.clone()),
        jobs: opts.jobs,
        sign: opts.sign.clone(),
        ..BuildOptions::default()
    };
    let built = build_project(&project, &build_opts)?;
    info!(
        "I: injected goauld -> {} (signed={})",
        built.output_apk.display(),
        built.signed
    );

    if !keep_project {
        let _ = std::fs::remove_dir_all(&project);
    }

    Ok(built.output_apk)
}

/// Mutate an already-decoded project: SO, loader DEX, manifest provider, doNotCompress.
pub fn apply_goauld_inject(project: &Path, agent_so: &Path) -> Result<()> {
    let lib_dir = project.join("lib").join(AGENT_ABI_DIR);
    std::fs::create_dir_all(&lib_dir)?;
    let dest_so = lib_dir.join(AGENT_SO_NAME);
    std::fs::copy(agent_so, &dest_so)?;
    info!(
        "I: packed agent → lib/{AGENT_ABI_DIR}/{AGENT_SO_NAME} (from {})",
        agent_so.display()
    );

    let dex_name = next_free_dex_name(project)?;
    let dex_path = project.join(&dex_name);
    std::fs::write(&dex_path, GOAULD_LOADER_DEX)?;
    info!("I: wrote loader {}", dex_path.display());

    let manifest_path = project.join("AndroidManifest.xml");
    if !manifest_path.is_file() {
        return Err(InjectError::Inject(
            "AndroidManifest.xml missing in project".into(),
        ));
    }
    let xml = std::fs::read_to_string(&manifest_path)?;
    let patched = insert_goauld_loader_provider(&xml);
    std::fs::write(&manifest_path, patched)?;
    info!("I: registered goauld.inject.LoaderProvider in manifest");

    let meta_path = project.join(META_FILENAME);
    let mut meta = ApkToolMeta::load(&meta_path)?;
    if !meta.doNotCompress.iter().any(|e| e == "so") {
        meta.doNotCompress.push("so".into());
        meta.save(&meta_path)?;
    }

    Ok(())
}

/// Mutate a VFS project: pack agent `.so`, loader DEX, provider, doNotCompress.
pub fn apply_goauld_inject_vfs(
    vfs: &mut dyn apk_patch_vfs::Vfs,
    project: &str,
    agent_so: &[u8],
) -> Result<()> {
    use apk_patch_vfs::{join_vfs, parent_vfs};

    let dest_so = join_vfs(project, &format!("lib/{AGENT_ABI_DIR}/{AGENT_SO_NAME}"));
    if let Some(parent) = parent_vfs(&dest_so) {
        vfs.create_dir_all(&parent)
            .map_err(|e| InjectError::Inject(e.to_string()))?;
    }
    vfs.write(&dest_so, agent_so)
        .map_err(|e| InjectError::Inject(e.to_string()))?;
    info!("I: packed agent → lib/{AGENT_ABI_DIR}/{AGENT_SO_NAME}");

    let dex_name = next_free_dex_name_vfs(vfs, project)?;
    let dex_path = join_vfs(project, &dex_name);
    vfs.write(&dex_path, GOAULD_LOADER_DEX)
        .map_err(|e| InjectError::Inject(e.to_string()))?;
    info!("I: wrote loader {dex_path}");

    let manifest_path = join_vfs(project, "AndroidManifest.xml");
    if !vfs.is_file(&manifest_path) {
        return Err(InjectError::Inject(
            "AndroidManifest.xml missing in project".into(),
        ));
    }
    let xml = vfs
        .read_to_string(&manifest_path)
        .map_err(|e| InjectError::Inject(e.to_string()))?;
    let patched = insert_goauld_loader_provider(&xml);
    vfs.write(&manifest_path, patched.as_bytes())
        .map_err(|e| InjectError::Inject(e.to_string()))?;
    info!("I: registered goauld.inject.LoaderProvider in manifest");

    let meta_path = join_vfs(project, META_FILENAME);
    let mut meta = ApkToolMeta::from_yaml(
        &vfs
            .read_to_string(&meta_path)
            .map_err(|e| InjectError::Inject(e.to_string()))?,
    )?;
    if !meta.doNotCompress.iter().any(|e| e == "so") {
        meta.doNotCompress.push("so".into());
        vfs.write(&meta_path, meta.to_yaml()?.as_bytes())
            .map_err(|e| InjectError::Inject(e.to_string()))?;
    }

    Ok(())
}

fn next_free_dex_name_vfs(vfs: &dyn apk_patch_vfs::Vfs, project: &str) -> Result<String> {
    use apk_patch_vfs::join_vfs;

    let mut used = std::collections::HashSet::new();
    if let Ok(entries) = vfs.read_dir(project) {
        for entry in entries {
            if is_classes_dex_name(&entry.name) {
                used.insert(entry.name);
            }
        }
    }
    let original = join_vfs(project, "original");
    if vfs.is_dir(&original) {
        if let Ok(entries) = vfs.read_dir(&original) {
            for entry in entries {
                if is_classes_dex_name(&entry.name) {
                    used.insert(entry.name);
                }
            }
        }
    }
    if let Ok(dirs) = apk_patch_dex::list_dex_dirs_vfs(vfs, project) {
        for d in dirs {
            used.insert(d.apk_dex_name);
        }
    }

    if !used.contains("classes.dex") {
        return Ok("classes.dex".into());
    }
    for n in 2u32..10_000 {
        let name = format!("classes{n}.dex");
        if !used.contains(&name) {
            return Ok(name);
        }
    }
    Err(InjectError::Inject("no free classesN.dex slot".into()))
}

fn next_free_dex_name(project: &Path) -> Result<String> {
    let mut used = std::collections::HashSet::new();
    for entry in std::fs::read_dir(project)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_classes_dex_name(&name) {
            used.insert(name);
        }
    }
    let original = project.join("original");
    if original.is_dir() {
        for entry in std::fs::read_dir(&original)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if is_classes_dex_name(&name) {
                used.insert(name);
            }
        }
    }
    // Also treat dex*/ directories as occupied slot names.
    if let Ok(dirs) = apk_patch_dex::list_dex_dirs(project) {
        for d in dirs {
            used.insert(d.apk_dex_name);
        }
    }

    if !used.contains("classes.dex") {
        return Ok("classes.dex".into());
    }
    for n in 2u32..10_000 {
        let name = format!("classes{n}.dex");
        if !used.contains(&name) {
            return Ok(name);
        }
    }
    Err(InjectError::Inject("no free classesN.dex slot".into()))
}

fn is_classes_dex_name(name: &str) -> bool {
    if name == "classes.dex" {
        return true;
    }
    let Some(stem) = name.strip_suffix(".dex") else {
        return false;
    };
    let Some(suffix) = stem.strip_prefix("classes") else {
        return false;
    };
    !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit())
}

fn tempfile_dir() -> Result<PathBuf> {
    let base = std::env::temp_dir().join(format!(
        "apk-patch-goauld-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&base)?;
    Ok(base)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loader_dex_embedded() {
        assert!(GOAULD_LOADER_DEX.len() > 100);
        assert!(GOAULD_LOADER_DEX.starts_with(b"dex\n"));
        let hay = GOAULD_LOADER_DEX;
        assert!(hay.windows(b"goauld_agent".len()).any(|w| w == b"goauld_agent"));
        assert!(hay
            .windows(b"Lgoauld/inject/LoaderProvider;".len())
            .any(|w| w == b"Lgoauld/inject/LoaderProvider;"));
    }

    #[test]
    fn next_dex_skips_existing() {
        let dir = tempfile_dir().unwrap();
        std::fs::write(dir.join("classes.dex"), b"dex\n").unwrap();
        std::fs::write(dir.join("classes2.dex"), b"dex\n").unwrap();
        assert_eq!(next_free_dex_name(&dir).unwrap(), "classes3.dex");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
