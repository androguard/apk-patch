//! VFS-backed APK build (shared by Path + MemVfs APIs).

use std::collections::HashMap;
use std::path::PathBuf;

use apk_patch_dex::{assemble_dex_from_vfs, list_dex_dirs_vfs, AssembleOptions};
use apk_patch_meta::{ApkToolMeta, META_FILENAME};
use apk_patch_project::{collect_build_entries_vfs, BuildEntry};
use apk_patch_resources::{build_arsc_from_vfs, BuildArscOptions};
use apk_patch_sign::sign_build_output;
use apk_patch_vfs::{join_vfs, parent_vfs, Vfs};
use apkparser::{ApkWriter, ZipEntry};
use axml_parser::encode_xml;
use log::{info, warn};

use crate::build::{BuildError, BuildOptions, BuildResult, Result};
use crate::manifest::{encode_manifest_to_axml, looks_like_binary_axml};
use crate::manifest_patch::{
    network_security_config_xml, patch_manifest_xml, ManifestBuildPatch,
};

/// `Instant` is unavailable on `wasm32-unknown-unknown`; timing is best-effort.
struct Tick {
    #[cfg(not(target_arch = "wasm32"))]
    start: std::time::Instant,
}

impl Tick {
    fn now() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            start: std::time::Instant::now(),
        }
    }

    fn secs_f64(&self) -> f64 {
        #[cfg(target_arch = "wasm32")]
        {
            0.0
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.start.elapsed().as_secs_f64()
        }
    }
}

/// Build a project from VFS. Writes `dist/` + `build/` into the VFS and returns APK bytes
/// (empty when `no_apk`).
pub fn build_project_vfs(
    vfs: &mut dyn Vfs,
    project: &str,
    options: &BuildOptions,
) -> Result<(Vec<u8>, BuildResult)> {
    let t_build = Tick::now();
    let meta_path = join_vfs(project, META_FILENAME);
    if !vfs.is_file(&meta_path) {
        return Err(BuildError::Build(format!(
            "missing {META_FILENAME} in {project}"
        )));
    }

    let meta = ApkToolMeta::from_yaml(
        &vfs
            .read_to_string(&meta_path)
            .map_err(|e| BuildError::Build(e.to_string()))?,
    )?;

    let dist_dir = join_vfs(project, "dist");
    let build_dir = join_vfs(project, "build");
    let build_apk_dir = join_vfs(&build_dir, "apk");
    vfs.create_dir_all(&build_dir)
        .map_err(|e| BuildError::Build(e.to_string()))?;

    let output_apk = options
        .output
        .as_ref()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|| join_vfs(&dist_dir, &meta.apkFileName));

    info!("I: building {project} → {output_apk}");

    // Incremental skip only for StdFs-style path existence with mtime — skip on MemVfs.
    // Force rebuild when using VFS bytes API unless caller sets force=false and file exists
    // with no way to compare mtimes; MemVfs has no mtimes so always rebuild.

    let mut manifest_patch = ManifestBuildPatch::new();
    manifest_patch.debuggable = options.debuggable;
    manifest_patch.net_sec_conf = options.net_sec_conf;

    if options.net_sec_conf {
        let xml_dir = join_vfs(project, "res/xml");
        vfs.create_dir_all(&xml_dir)
            .map_err(|e| BuildError::Build(e.to_string()))?;
        vfs.write(
            &join_vfs(&xml_dir, "network_security_config.xml"),
            network_security_config_xml().as_bytes(),
        )
        .map_err(|e| BuildError::Build(e.to_string()))?;
    }

    let text_manifest_path = join_vfs(project, "AndroidManifest.xml");
    let patched_manifest_path = join_vfs(&build_dir, "AndroidManifest.xml");
    if vfs.is_file(&text_manifest_path) {
        let raw = vfs
            .read_to_string(&text_manifest_path)
            .map_err(|e| BuildError::Build(e.to_string()))?;
        let patched = patch_manifest_xml(&raw, &meta, &manifest_patch);
        vfs.write(&patched_manifest_path, patched.as_bytes())
            .map_err(|e| BuildError::Build(e.to_string()))?;
    } else if vfs.is_file(&join_vfs(project, "original/AndroidManifest.xml")) {
        vfs.copy(
            &join_vfs(project, "original/AndroidManifest.xml"),
            &patched_manifest_path,
        )
        .map_err(|e| BuildError::Build(e.to_string()))?;
    }

    let used_aapt2 = false;
    let mut used_rust_arsc = false;
    let mut rebuilt_arsc: Option<Vec<u8>> = None;
    let res_dir = join_vfs(project, "res");
    let can_rebuild = !options.skip_aapt2
        && !options.copy_original
        && vfs.is_dir(&res_dir)
        && vfs.is_file(&join_vfs(&res_dir, "values/public.xml"));
    let want_rebuild = options.use_aapt2 || options.rebuild_resources;

    // aapt2 is native-only; in VFS/WASM builds we never spawn it.
    if can_rebuild && options.use_aapt2 {
        warn!("W: aapt2 not available in VFS/WASM build path; trying pure-Rust ARSC builder");
    }

    if can_rebuild && (options.rebuild_resources || options.use_aapt2) {
        info!("I: rebuilding resources.arsc (pure-Rust)…");
        match build_arsc_from_vfs(
            vfs,
            project,
            &BuildArscOptions {
                package_id: meta.resourcesInfo.packageId,
                package_name: meta.resourcesInfo.packageName.clone(),
            },
        ) {
            Ok(built) => {
                vfs.write(&join_vfs(&build_dir, "resources.arsc"), &built.arsc)
                    .map_err(|e| BuildError::Build(e.to_string()))?;
                rebuilt_arsc = Some(built.arsc);
                used_rust_arsc = true;
                info!(
                    "I: pure-Rust ARSC builder wrote {} entries",
                    built.entry_count
                );
            }
            Err(e) => {
                warn!("W: pure-Rust ARSC builder failed ({e}); packing original/resources.arsc");
            }
        }
    } else if !can_rebuild || !want_rebuild {
        info!("I: skipping resource rebuild (using original resources.arsc)");
    }

    let assemble_opts = AssembleOptions {
        jobs: options.jobs,
    };
    let dex_dirs = list_dex_dirs_vfs(vfs, project)?;
    let mut assembled_dex: HashMap<String, Vec<u8>> = HashMap::new();
    for entry in &dex_dirs {
        let dex_bytes = assemble_dex_from_vfs(vfs, project, entry, &assemble_opts)?;
        assembled_dex.insert(entry.apk_dex_name.clone(), dex_bytes);
    }

    info!("I: collecting APK entries…");
    let mut entries = collect_build_entries_vfs(vfs, project, &meta, &assembled_dex)?;

    entries.retain(|e| e.name != "AndroidManifest.xml" && e.name != "resources.arsc");
    if let Some(arsc) = rebuilt_arsc {
        entries.push(BuildEntry {
            name: "resources.arsc".into(),
            data: arsc,
            compress: false,
        });
    } else {
        let original_arsc = join_vfs(project, "original/resources.arsc");
        let root_arsc = join_vfs(project, "resources.arsc");
        let arsc_path = if vfs.is_file(&original_arsc) {
            original_arsc
        } else {
            root_arsc
        };
        if vfs.is_file(&arsc_path) {
            entries.push(BuildEntry {
                name: "resources.arsc".into(),
                data: vfs
                    .read(&arsc_path)
                    .map_err(|e| BuildError::Build(e.to_string()))?,
                compress: false,
            });
        }
        if let Some(raw_entries) = load_res_raw_entries_vfs(vfs, project)? {
            entries.retain(|e| !e.name.starts_with("res/"));
            let n = raw_entries.len();
            entries.extend(raw_entries);
            info!("I: packing {n} res entries from original/res-raw.zip (exact APK case)");
        }
    }

    if options.copy_original {
        entries.retain(|e| e.name != "AndroidManifest.xml" && !e.name.starts_with("META-INF/"));
        let orig_manifest = join_vfs(project, "original/AndroidManifest.xml");
        if vfs.is_file(&orig_manifest) {
            entries.push(BuildEntry {
                name: "AndroidManifest.xml".into(),
                data: vfs
                    .read(&orig_manifest)
                    .map_err(|e| BuildError::Build(e.to_string()))?,
                compress: true,
            });
        }
        if !options.sign.enabled {
            let meta_inf = join_vfs(project, "original/META-INF");
            if vfs.is_dir(&meta_inf) {
                for path in vfs
                    .walk_files(&meta_inf)
                    .map_err(|e| BuildError::Build(e.to_string()))?
                {
                    let rel = path
                        .strip_prefix(&meta_inf)
                        .map(|s| s.trim_start_matches('/'))
                        .unwrap_or(path.as_str());
                    entries.push(BuildEntry {
                        name: format!("META-INF/{rel}"),
                        data: vfs
                            .read(&path)
                            .map_err(|e| BuildError::Build(e.to_string()))?,
                        compress: true,
                    });
                }
            }
        }
    } else if vfs.is_file(&patched_manifest_path) {
        entries.retain(|e| e.name != "AndroidManifest.xml");
        entries.push(BuildEntry {
            name: "AndroidManifest.xml".into(),
            data: vfs
                .read(&patched_manifest_path)
                .map_err(|e| BuildError::Build(e.to_string()))?,
            compress: true,
        });
    }

    entries.sort_by(|a, b| a.name.cmp(&b.name));
    {
        let mut map = HashMap::new();
        for e in entries {
            map.insert(e.name.clone(), e);
        }
        entries = map.into_values().collect();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
    }

    if entries.is_empty() {
        return Err(BuildError::Build("no entries to pack".into()));
    }

    info!("I: writing {} intermediates to {build_apk_dir}…", entries.len());
    vfs.create_dir_all(&build_apk_dir)
        .map_err(|e| BuildError::Build(e.to_string()))?;
    for entry in &entries {
        let dest = join_vfs(&build_apk_dir, &entry.name);
        if let Some(parent) = parent_vfs(&dest) {
            vfs.create_dir_all(&parent)
                .map_err(|e| BuildError::Build(e.to_string()))?;
        }
        let data = encode_pack_entry(entry, options.copy_original)?;
        vfs.write(&dest, &data)
            .map_err(|e| BuildError::Build(e.to_string()))?;
    }

    if options.no_apk {
        info!(
            "I: --no-apk: wrote {} entries ({:.1}s)",
            entries.len(),
            t_build.secs_f64()
        );
        return Ok((
            Vec::new(),
            BuildResult {
                output_apk: PathBuf::from(&build_apk_dir),
                entry_count: entries.len(),
                signed: false,
                incremental_skipped: false,
                used_aapt2,
                used_rust_arsc,
            },
        ));
    }

    info!("I: packing ZIP ({} entries)…", entries.len());
    let mut writer = ApkWriter::new();
    for entry in &entries {
        let data = encode_pack_entry(entry, options.copy_original)?;
        writer.add_entry(&entry.name, &data, entry.compress);
    }
    let unsigned = writer.finish()?;

    if options.sign.enabled {
        info!(
            "I: signing APK (v1={}, v2={}, v3={})…",
            options.sign.v1, options.sign.v2, options.sign.v3
        );
    } else {
        warn!("W: --no-sign: APK will not install on device (no certificates)");
    }
    let signed_bytes = sign_build_output(&unsigned, &options.sign)?;

    vfs.create_dir_all(&dist_dir)
        .map_err(|e| BuildError::Build(e.to_string()))?;
    if let Some(parent) = parent_vfs(&output_apk) {
        vfs.create_dir_all(&parent)
            .map_err(|e| BuildError::Build(e.to_string()))?;
    }
    vfs.write(&output_apk, &signed_bytes)
        .map_err(|e| BuildError::Build(e.to_string()))?;
    info!(
        "I: wrote {output_apk} ({} bytes, {:.1}s total)",
        signed_bytes.len(),
        t_build.secs_f64()
    );

    Ok((
        signed_bytes,
        BuildResult {
            output_apk: PathBuf::from(&output_apk),
            entry_count: entries.len(),
            signed: options.sign.enabled,
            incremental_skipped: false,
            used_aapt2,
            used_rust_arsc,
        },
    ))
}

fn load_res_raw_entries_vfs(vfs: &dyn Vfs, project: &str) -> Result<Option<Vec<BuildEntry>>> {
    let path = join_vfs(project, "original/res-raw.zip");
    if !vfs.is_file(&path) {
        return Ok(None);
    }
    let bytes = vfs
        .read(&path)
        .map_err(|e| BuildError::Build(e.to_string()))?;
    let zip = ZipEntry::parse(&bytes).map_err(|e| BuildError::Build(format!("res-raw.zip: {e}")))?;
    let mut out = Vec::new();
    for name in zip.namelist() {
        if !name.starts_with("res/") || name.ends_with('/') {
            continue;
        }
        let data = zip
            .read_to_vec(name)
            .map_err(|e| BuildError::Build(format!("res-raw.zip {name}: {e}")))?;
        let compress = !name.ends_with(".png")
            && !name.ends_with(".jpg")
            && !name.ends_with(".jpeg")
            && !name.ends_with(".webp")
            && !name.ends_with(".gif")
            && !name.ends_with(".9.png");
        out.push(BuildEntry {
            name: name.clone(),
            data,
            compress,
        });
    }
    if out.is_empty() {
        Ok(None)
    } else {
        Ok(Some(out))
    }
}

fn encode_pack_entry(entry: &BuildEntry, copy_original: bool) -> Result<Vec<u8>> {
    if entry.name == "AndroidManifest.xml" && !copy_original {
        return encode_manifest_to_axml(&entry.data).map_err(|e| BuildError::Build(e.to_string()));
    }
    if entry.name.starts_with("res/")
        && entry.name.ends_with(".xml")
        && !entry.name.contains("/values")
        && !looks_like_binary_axml(&entry.data)
    {
        match encode_xml(&entry.data) {
            Ok(bin) => return Ok(bin),
            Err(e) => {
                warn!("W: AXML encode failed for {}: {e}; packing text", entry.name);
            }
        }
    }
    Ok(entry.data.clone())
}
