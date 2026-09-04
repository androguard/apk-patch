use std::path::{Path, PathBuf};

use apk_patch_dex::{dex_dir_name, emit_dex_to_dir, is_odex, is_odex_bytes, EmitOptions};
use apk_patch_meta::{ApkToolMeta, META_FILENAME};
use apk_patch_project::{decode_target, project_path, write_entry, DecodeTarget};
use apk_patch_resources::{
    decode_nine_patch_png, decode_res_xml, decode_resources_arsc, is_nine_patch_png,
    should_decode_res_xml, DecodeResOptions, ResResolveMode,
};
use apkparser::{Apk, ApkOptions, ApkWriter};
use thiserror::Error;

use crate::manifest::decode_manifest_to_xml;

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
            // Match Apktool: decode resources by default.
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

pub fn decode_apk(apk_path: &Path, options: &DecodeOptions) -> Result<DecodeResult> {
    let apk_name = apk_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| DecodeError::Decode("invalid apk path".into()))?;

    let output_dir = options.output.clone().unwrap_or_else(|| {
        apk_path
            .parent()
            .unwrap_or(Path::new("."))
            .join(apk_name.trim_end_matches(".apk"))
    });

    if output_dir.exists() {
        if options.force {
            std::fs::remove_dir_all(&output_dir)?;
        } else {
            return Err(DecodeError::Decode(format!(
                "output directory already exists: {} (use -f to overwrite)",
                output_dir.display()
            )));
        }
    }
    std::fs::create_dir_all(&output_dir)?;

    let apk = Apk::from_path(apk_path, ApkOptions::AXML.with_signature(false))?;
    let mut meta = ApkToolMeta::new(apk_name);
    if let Some(tag) = &options.frame_tag {
        meta.usesFramework.tag = Some(tag.clone());
    }
    if let Some(manifest) = apk.get_android_manifest() {
        meta.sdkInfo.minSdkVersion = manifest.min_sdk_version.map(|v| v.to_string());
        meta.sdkInfo.targetSdkVersion = manifest.target_sdk_version.map(|v| v.to_string());
        meta.versionInfo.versionCode = manifest.version_code;
        meta.versionInfo.versionName = manifest.version_name.clone();
    }

    let files: Vec<String> = apk.get_files().to_vec();
    let mut count = 0usize;
    let mut dex_class_count = 0usize;
    // Exact-case binary res/* for rebuild on case-insensitive filesystems (macOS/Windows).
    let mut res_raw = ApkWriter::new();
    let mut res_raw_count = 0usize;

    let emit_opts = EmitOptions {
        include_debug: !options.no_debug_info,
        jobs: options.jobs,
    };

    for name in &files {
        if options.only_manifest && name != "AndroidManifest.xml" {
            continue;
        }
        if options.no_assets && name.starts_with("assets/") {
            continue;
        }

        if apk_patch_project::is_dex_entry(name) {
            if options.only_manifest {
                continue;
            }
            if is_odex(name) {
                return Err(DecodeError::Decode(
                    "Cannot disassemble an odex file without deodexing".into(),
                ));
            }
            let data = apk.get_file(name)?;
            if is_odex_bytes(&data) {
                return Err(DecodeError::Decode(
                    "Cannot disassemble an odex file without deodexing".into(),
                ));
            }

            if !should_disassemble_dex(name, options) {
                if options.no_src {
                    let target = decode_target(name);
                    write_entry(&output_dir, &target, &data)?;
                    count += 1;
                }
                continue;
            }

            write_entry(
                &output_dir,
                &DecodeTarget::Original(name.clone()),
                &data,
            )?;
            count += 1;

            if !options.no_src {
                let dex_dir = output_dir.join(dex_dir_name(name));
                std::fs::create_dir_all(&dex_dir)?;
                dex_class_count += emit_dex_to_dir(&data, name, &dex_dir, &emit_opts)?;
            } else {
                let target = decode_target(name);
                write_entry(&output_dir, &target, &data)?;
            }
            continue;
        }

        if name == "resources.arsc" {
            let data = apk.get_file(name)?;
            // Always keep binary ARSC for rebuild.
            write_entry(
                &output_dir,
                &DecodeTarget::Original("resources.arsc".into()),
                &data,
            )?;
            count += 1;

            if options.no_res || options.only_manifest {
                write_entry(&output_dir, &DecodeTarget::Root(name.clone()), &data)?;
            } else {
                let res_opts = DecodeResOptions {
                    keep_broken: options.keep_broken_res,
                    resolve_mode: options.res_resolve_mode,
                    ignore_raw_values: options.ignore_raw_values,
                };
                match decode_resources_arsc(&data, &output_dir, &res_opts) {
                    Ok(res) => {
                        if let Some(pkg_id) = res.package_id {
                            meta.resourcesInfo.packageId = Some(pkg_id);
                        }
                        if let Some(name) = res.package_names.first() {
                            meta.resourcesInfo.packageName = Some(name.clone());
                        }
                        if !meta.usesFramework.ids.contains(&1) {
                            meta.usesFramework.ids.push(1);
                        }
                    }
                    Err(e) if options.keep_broken_res => {
                        log::warn!("W: resource decode failed, keeping raw arsc: {e}");
                        write_entry(&output_dir, &DecodeTarget::Root(name.clone()), &data)?;
                    }
                    Err(e) => return Err(e.into()),
                }
            }
            continue;
        }

        let data = apk.get_file(name)?;
        let target = decode_target(name);

        // Preserve original APK bytes/paths for res/* (case-sensitive ZIP names).
        if name.starts_with("res/") && !options.no_res {
            res_raw.add_entry(name.clone(), &data, false);
            res_raw_count += 1;
        }

        // When decoding resources, still copy binary/file res entries.
        if matches!(target, DecodeTarget::Res(_)) && options.no_res {
            // With -r, keep raw tree under unknown? Apktool keeps resources.arsc only.
            // File-based res/* still copied as-is when -r (Apktool leaves them in APK).
            // Actually with -r Apktool does NOT extract res/ folder files separately
            // beyond keeping arsc. We'll skip extracting individual res files when -r.
            continue;
        }

        // Decode binary res XML (layouts, menus, drawables, …) to text when possible.
        // Decode compiled nine-patch PNGs to editable bordered `.9.png`.
        let data = if !options.no_res {
            if let DecodeTarget::Res(ref rel) = target {
                if should_decode_res_xml(rel) {
                    if let Some(xml) = decode_res_xml(&data) {
                        write_entry(&output_dir, &target, &xml)?;
                        count += 1;
                        continue;
                    }
                }
                if is_nine_patch_png(rel) {
                    match decode_nine_patch_png(&data) {
                        Ok(decoded) => decoded,
                        Err(e) if options.keep_broken_res => {
                            log::warn!("W: nine-patch decode failed for {rel}: {e}");
                            data
                        }
                        Err(e) => {
                            return Err(DecodeError::Decode(format!(
                                "nine-patch decode failed for {rel}: {e}"
                            )));
                        }
                    }
                } else {
                    data
                }
            } else {
                data
            }
        } else {
            data
        };

        write_entry(&output_dir, &target, &data)?;
        count += 1;

        if name == "AndroidManifest.xml" {
            let root_manifest =
                project_path(&output_dir, &DecodeTarget::Root("AndroidManifest.xml".into()));
            if let Some(parent) = root_manifest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let manifest_xml = decode_manifest_to_xml(&data)?;
            std::fs::write(&root_manifest, &manifest_xml)?;
        }
    }

    if res_raw_count > 0 {
        let raw_zip = output_dir.join("original/res-raw.zip");
        if let Some(parent) = raw_zip.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = res_raw
            .finish()
            .map_err(|e| DecodeError::Decode(format!("res-raw.zip: {e}")))?;
        std::fs::write(&raw_zip, bytes)?;
        log::info!(
            "I: wrote {} exact-case res entries → {}",
            res_raw_count,
            raw_zip.display()
        );
    }

    let _ = &options.frame_path;
    meta.save(&output_dir.join(META_FILENAME))?;

    Ok(DecodeResult {
        output_dir,
        entry_count: count,
        dex_class_count,
    })
}

fn should_disassemble_dex(name: &str, options: &DecodeOptions) -> bool {
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
