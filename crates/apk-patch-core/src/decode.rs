use std::path::PathBuf;
#[cfg(feature = "native-fs")]
use std::path::Path;

use apk_patch_resources::ResResolveMode;
use log::info;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DecodeError {
    #[error(transparent)]
    Apk(#[from] apkparser::Error),
    #[error(transparent)]
    Meta(#[from] apk_patch_meta::MetaError),
    #[error(transparent)]
    Project(#[from] apk_patch_project::ProjectError),
    #[error(transparent)]
    Dex(#[from] apk_patch_dex::DexError),
    #[error(transparent)]
    Resources(#[from] apk_patch_resources::ResourceError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("decode error: {0}")]
    Decode(String),
}

pub type Result<T> = std::result::Result<T, DecodeError>;

#[derive(Debug, Clone)]
pub struct DecodeOptions {
    pub force: bool,
    pub no_src: bool,
    pub no_res: bool,
    pub no_assets: bool,
    pub all_src: bool,
    pub no_debug_info: bool,
    pub only_manifest: bool,
    pub keep_broken_res: bool,
    pub res_resolve_mode: ResResolveMode,
    pub ignore_raw_values: bool,
    pub frame_path: Option<PathBuf>,
    pub frame_tag: Option<String>,
    pub jobs: usize,
    pub output: Option<PathBuf>,
}

impl Default for DecodeOptions {
    fn default() -> Self {
        Self {
            force: false,
            no_src: false,
            no_res: false,
            no_assets: false,
            all_src: false,
            no_debug_info: false,
            only_manifest: false,
            keep_broken_res: false,
            res_resolve_mode: ResResolveMode::Default,
            ignore_raw_values: false,
            frame_path: None,
            frame_tag: None,
            jobs: num_cpus(),
            output: None,
        }
    }
}

fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(8)
}

#[derive(Debug)]
pub struct DecodeResult {
    pub output_dir: PathBuf,
    pub entry_count: usize,
    pub dex_class_count: usize,
}

/// Path-based decode (host filesystem). Requires the `native-fs` feature.
///
/// Accepts plain `.apk`, APKPure `.xapk`, and APKMirror `.apkm` containers.
/// For split containers, the base APK is decoded into the project root and non-base
/// members are preserved under `container/` for a faithful rebuild.
#[cfg(feature = "native-fs")]
pub fn decode_apk(apk_path: &Path, options: &DecodeOptions) -> Result<DecodeResult> {
    let apk_name = apk_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| DecodeError::Decode("invalid apk path".into()))?;

    let output_dir = options.output.clone().unwrap_or_else(|| {
        apk_path
            .parent()
            .unwrap_or(Path::new("."))
            .join(crate::container::strip_package_extension(apk_name))
    });

    let bytes = std::fs::read(apk_path)?;

    if let Some(pkg) = crate::container::SplitPackage::open(&bytes)? {
        info!(
            "I: detected {} container (base={}, {} members)",
            pkg.format.as_str(),
            pkg.base_apk,
            pkg.members.len()
        );
        let base_bytes = pkg.base_apk_bytes()?;
        let mut vfs = apk_patch_vfs::StdFs::new();
        let out = output_dir.to_string_lossy().replace('\\', "/");
        // Decode using the outer package name so apkpatch.yml / dist keep .xapk/.apkm.
        let mut result =
            crate::decode_vfs::decode_apk_vfs(&base_bytes, apk_name, &out, options, &mut vfs)?;
        pkg.write_preserved_members(&output_dir)?;
        crate::container::stamp_container_meta(
            &output_dir,
            apk_name,
            pkg.format,
            pkg.to_meta(),
        )?;
        // Count preserved container members toward entry_count for visibility.
        result.entry_count += pkg.members.len().saturating_sub(1);
        info!(
            "I: preserved {} non-base container member(s) under {}/",
            pkg.members.len().saturating_sub(1),
            apk_patch_meta::CONTAINER_DIR
        );
        return Ok(result);
    }

    let mut vfs = apk_patch_vfs::StdFs::new();
    let out = output_dir.to_string_lossy().replace('\\', "/");
    crate::decode_vfs::decode_apk_vfs(&bytes, apk_name, &out, options, &mut vfs)
}

pub(crate) fn should_disassemble_dex(name: &str, options: &DecodeOptions) -> bool {
    if options.all_src {
        return true;
    }
    if name == "classes.dex" {
        return true;
    }
    let stem = name.strip_suffix(".dex").unwrap_or(name);
    if stem.starts_with("classes") {
        let suffix = stem.strip_prefix("classes").unwrap_or("");
        if suffix.is_empty() {
            return true;
        }
        if suffix.parse::<u32>().is_ok() {
            return true;
        }
    }
    false
}
