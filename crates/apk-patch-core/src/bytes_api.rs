//! Bytes-oriented decode/build/inject APIs (MemVfs, WASM-friendly).

use apk_patch_vfs::MemVfs;

use crate::build::BuildOptions;
use crate::build_vfs::build_project_vfs;
use crate::decode::{DecodeError, DecodeOptions};
use crate::decode_vfs::decode_apk_vfs;
use crate::inject_goauld::{apply_goauld_inject_vfs, InjectGoauldOptions};

const PROJECT_ROOT: &str = "project";

#[derive(Debug)]
pub struct BytesDecodeResult {
    pub vfs: MemVfs,
    pub project_root: String,
    pub entry_count: usize,
    pub dex_class_count: usize,
}

#[derive(Debug)]
pub struct BytesBuildResult {
    pub apk_bytes: Vec<u8>,
    pub entry_count: usize,
    pub signed: bool,
    pub used_rust_arsc: bool,
}

/// Decode an APK into an in-memory project tree.
pub fn decode_apk_bytes(apk: &[u8], apk_name: &str, options: &DecodeOptions) -> Result<BytesDecodeResult, DecodeError> {
    let mut opts = options.clone();
    opts.force = true;
    let mut vfs = MemVfs::new();
    let result = decode_apk_vfs(apk, apk_name, PROJECT_ROOT, &opts, &mut vfs)?;
    Ok(BytesDecodeResult {
        vfs,
        project_root: PROJECT_ROOT.into(),
        entry_count: result.entry_count,
        dex_class_count: result.dex_class_count,
    })
}

/// Build a MemVfs project into a signed (or unsigned) APK.
pub fn build_project_bytes(
    vfs: &mut MemVfs,
    project_root: &str,
    options: &BuildOptions,
) -> Result<BytesBuildResult, crate::build::BuildError> {
    let mut opts = options.clone();
    // Browser-safe defaults when aapt2 is requested.
    if opts.use_aapt2 {
        opts.use_aapt2 = false;
        if !opts.rebuild_resources {
            opts.rebuild_resources = true;
        }
    }
    opts.skip_aapt2 = true;
    let (apk_bytes, result) = build_project_vfs(vfs, project_root, &opts)?;
    Ok(BytesBuildResult {
        apk_bytes,
        entry_count: result.entry_count,
        signed: result.signed,
        used_rust_arsc: result.used_rust_arsc,
    })
}

/// Decode → inject goauld agent `.so` + loader → rebuild+sign.
pub fn inject_goauld_bytes(
    apk: &[u8],
    agent_so: &[u8],
    apk_name: &str,
    options: &InjectGoauldOptions,
) -> anyhow::Result<Vec<u8>> {
    let decode_opts = DecodeOptions {
        force: true,
        jobs: options.jobs,
        ..DecodeOptions::default()
    };
    let mut decoded = decode_apk_bytes(apk, apk_name, &decode_opts)?;
    apply_goauld_inject_vfs(&mut decoded.vfs, &decoded.project_root, agent_so)?;
    let build_opts = BuildOptions {
        force: true,
        jobs: options.jobs,
        sign: options.sign.clone(),
        skip_aapt2: true,
        ..BuildOptions::default()
    };
    let built = build_project_bytes(&mut decoded.vfs, &decoded.project_root, &build_opts)?;
    Ok(built.apk_bytes)
}
