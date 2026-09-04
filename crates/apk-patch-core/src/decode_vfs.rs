//! VFS-backed APK decode (shared by Path + MemVfs APIs).

use apk_patch_dex::{dex_dir_name, emit_dex_to_vfs, is_odex, is_odex_bytes, EmitOptions};
use apk_patch_meta::{ApkToolMeta, META_FILENAME};
use apk_patch_project::{
    decode_target, is_dex_entry, project_path_vfs, write_entry_vfs, DecodeTarget,
};
use apk_patch_resources::{
    decode_nine_patch_png, decode_res_xml, decode_resources_arsc_vfs, is_nine_patch_png,
    should_decode_res_xml, DecodeResOptions,
};
use apk_patch_vfs::{join_vfs, parent_vfs, Vfs};
use apkparser::{Apk, ApkOptions, ApkWriter};

use crate::decode::{should_disassemble_dex, DecodeError, DecodeOptions, DecodeResult, Result};
use crate::manifest::decode_manifest_to_xml;

/// Decode APK bytes into a VFS project rooted at `output_dir`.
pub fn decode_apk_vfs(
    apk_bytes: &[u8],
    apk_name: &str,
    output_dir: &str,
    options: &DecodeOptions,
    vfs: &mut dyn Vfs,
) -> Result<DecodeResult> {
    if vfs.exists(output_dir) {
        if options.force {
            vfs.remove_dir_all(output_dir)
                .map_err(|e| DecodeError::Decode(e.to_string()))?;
        } else {
            return Err(DecodeError::Decode(format!(
                "output directory already exists: {output_dir} (use -f to overwrite)"
            )));
        }
    }
    vfs.create_dir_all(output_dir)
        .map_err(|e| DecodeError::Decode(e.to_string()))?;

    let apk = Apk::from_bytes(apk_bytes, ApkOptions::AXML.with_signature(false))?;
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

        if is_dex_entry(name) {
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
                    write_entry_vfs(vfs, output_dir, &target, &data)?;
                    count += 1;
                }
                continue;
            }

            write_entry_vfs(
                vfs,
                output_dir,
                &DecodeTarget::Original(name.clone()),
                &data,
            )?;
            count += 1;

            if !options.no_src {
                let dex_dir = join_vfs(output_dir, &dex_dir_name(name));
                vfs.create_dir_all(&dex_dir)
                    .map_err(|e| DecodeError::Decode(e.to_string()))?;
                dex_class_count += emit_dex_to_vfs(&data, name, vfs, &dex_dir, &emit_opts)?;
            } else {
                let target = decode_target(name);
                write_entry_vfs(vfs, output_dir, &target, &data)?;
            }
            continue;
        }

        if name == "resources.arsc" {
            let data = apk.get_file(name)?;
            write_entry_vfs(
                vfs,
                output_dir,
                &DecodeTarget::Original("resources.arsc".into()),
                &data,
            )?;
            count += 1;

            if options.no_res || options.only_manifest {
                write_entry_vfs(vfs, output_dir, &DecodeTarget::Root(name.clone()), &data)?;
            } else {
                let res_opts = DecodeResOptions {
                    keep_broken: options.keep_broken_res,
                    resolve_mode: options.res_resolve_mode,
                    ignore_raw_values: options.ignore_raw_values,
                };
                match decode_resources_arsc_vfs(&data, vfs, output_dir, &res_opts) {
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
                        write_entry_vfs(vfs, output_dir, &DecodeTarget::Root(name.clone()), &data)?;
                    }
                    Err(e) => return Err(e.into()),
                }
            }
            continue;
        }

        let data = apk.get_file(name)?;
        let target = decode_target(name);

        if name.starts_with("res/") && !options.no_res {
            res_raw.add_entry(name.clone(), &data, false);
            res_raw_count += 1;
        }

        if matches!(target, DecodeTarget::Res(_)) && options.no_res {
            continue;
        }

        let data = if !options.no_res {
            if let DecodeTarget::Res(ref rel) = target {
                if should_decode_res_xml(rel) {
                    if let Some(xml) = decode_res_xml(&data) {
                        write_entry_vfs(vfs, output_dir, &target, &xml)?;
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

        write_entry_vfs(vfs, output_dir, &target, &data)?;
        count += 1;

        if name == "AndroidManifest.xml" {
            let root_manifest =
                project_path_vfs(output_dir, &DecodeTarget::Root("AndroidManifest.xml".into()));
            if let Some(parent) = parent_vfs(&root_manifest) {
                vfs.create_dir_all(&parent)
                    .map_err(|e| DecodeError::Decode(e.to_string()))?;
            }
            let manifest_xml = decode_manifest_to_xml(&data)?;
            vfs.write(&root_manifest, &manifest_xml)
                .map_err(|e| DecodeError::Decode(e.to_string()))?;
        }
    }

    if res_raw_count > 0 {
        let raw_zip = join_vfs(output_dir, "original/res-raw.zip");
        if let Some(parent) = parent_vfs(&raw_zip) {
            vfs.create_dir_all(&parent)
                .map_err(|e| DecodeError::Decode(e.to_string()))?;
        }
        let bytes = res_raw
            .finish()
            .map_err(|e| DecodeError::Decode(format!("res-raw.zip: {e}")))?;
        vfs.write(&raw_zip, &bytes)
            .map_err(|e| DecodeError::Decode(e.to_string()))?;
        log::info!("I: wrote {res_raw_count} exact-case res entries → {raw_zip}");
    }

    let _ = &options.frame_path;
    let meta_path = join_vfs(output_dir, META_FILENAME);
    vfs.write(&meta_path, meta.to_yaml()?.as_bytes())
        .map_err(|e| DecodeError::Decode(e.to_string()))?;

    Ok(DecodeResult {
        output_dir: std::path::PathBuf::from(output_dir),
        entry_count: count,
        dex_class_count,
    })
}
