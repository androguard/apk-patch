use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

use apk_patch_dex::{assemble_dex_from_project, list_dex_dirs, AssembleOptions};
use apk_patch_meta::ApkToolMeta;
use apk_patch_project::{collect_build_entries, BuildEntry};
#[cfg(feature = "aapt2")]
use apk_patch_framework::{get_framework_apk, FrameworkOptions};
#[cfg(feature = "aapt2")]
use apk_patch_resources::{
    aapt2_compile, aapt2_link, find_aapt2, find_android_jar, Aapt2CompileOptions, Aapt2LinkOptions,
};
use apk_patch_resources::{build_arsc_from_project, BuildArscOptions};
use apk_patch_sign::{sign_build_output, BuildSignConfig};
use apkparser::{ApkWriter, ZipEntry};
use axml_parser::encode_xml;
use log::{debug, info, warn};
use thiserror::Error;
use walkdir::WalkDir;

use crate::manifest::{encode_manifest_to_axml, looks_like_binary_axml};
use crate::manifest_patch::{
    network_security_config_xml, patch_manifest_xml, ManifestBuildPatch,
};

#[derive(Error, Debug)]
pub enum BuildError {
    #[error(transparent)]
    Meta(#[from] apk_patch_meta::MetaError),
    #[error(transparent)]
    Project(#[from] apk_patch_project::ProjectError),
    #[error(transparent)]
    Apk(#[from] apkparser::Error),
    #[error(transparent)]
    Sign(#[from] apk_patch_sign::Error),
    #[error(transparent)]
    Dex(#[from] apk_patch_dex::DexError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("build error: {0}")]
    Build(String),
}

pub type Result<T> = std::result::Result<T, BuildError>;

#[derive(Debug, Clone)]
pub struct BuildOptions {
    pub force: bool,
    pub output: Option<PathBuf>,
    pub jobs: usize,
    pub sign: BuildSignConfig,
    /// Custom aapt2 binary (`--aapt`).
    pub aapt: Option<PathBuf>,
    /// Framework directory for `-I` includes (`-p` / `--frame-path`).
    pub frame_path: Option<PathBuf>,
    pub frame_tag: Option<String>,
    /// Set `android:debuggable="true"`.
    pub debuggable: bool,
    /// Inject permissive network security config.
    pub net_sec_conf: bool,
    /// Disable PNG crunching in aapt2 compile.
    pub no_crunch: bool,
    /// Write intermediates to `build/apk/` only; skip final APK zip.
    pub no_apk: bool,
    /// Prefer `original/` manifest + META-INF over rebuilt ones.
    pub copy_original: bool,
    /// Prefer packing `original/resources.arsc` (skip pure-Rust and aapt2 rebuild).
    pub skip_aapt2: bool,
    /// Prefer aapt2 over the pure-Rust ARSC builder when available.
    pub use_aapt2: bool,
    /// Force pure-Rust `resources.arsc` rebuild (off by default; can break install).
    pub rebuild_resources: bool,
}

impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            force: false,
            output: None,
            jobs: num_cpus(),
            sign: BuildSignConfig::default(),
            aapt: None,
            frame_path: None,
            frame_tag: None,
            debuggable: false,
            net_sec_conf: false,
            no_crunch: false,
            no_apk: false,
            copy_original: false,
            skip_aapt2: false,
            use_aapt2: false,
            rebuild_resources: false,
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
pub struct BuildResult {
    pub output_apk: PathBuf,
    pub entry_count: usize,
    pub signed: bool,
    /// True when an existing APK was kept because inputs were not newer.
    pub incremental_skipped: bool,
    /// True when aapt2 compile/link was used.
    pub used_aapt2: bool,
    /// True when the pure-Rust ARSC builder was used.
    pub used_rust_arsc: bool,
}

#[cfg(feature = "native-fs")]
pub fn build_project(project: &Path, options: &BuildOptions) -> Result<BuildResult> {
    let t_build = Instant::now();
    let meta_path = match apk_patch_meta::find_meta_path(project) {
        Ok(p) => p,
        Err(e) => return Err(BuildError::Build(e.to_string())),
    };

    let meta = ApkToolMeta::load(&meta_path)?;
    let dist_dir = project.join("dist");
    let build_dir = project.join("build");
    let build_apk_dir = build_dir.join("apk");
    std::fs::create_dir_all(&build_dir)?;

    let output_apk = options
        .output
        .clone()
        .unwrap_or_else(|| dist_dir.join(&meta.apkFileName));

    info!(
        "I: building {} → {}",
        project.display(),
        if options.no_apk {
            build_apk_dir.display().to_string()
        } else {
            output_apk.display().to_string()
        }
    );

    // Incremental: skip full rebuild when output is newer than all inputs (unless -f).
    // Otherwise overwrite dist APK (Apktool parity; `-f` forces rebuild even if up-to-date).
    if !options.force && !options.no_apk && output_apk.is_file() {
        if let (Ok(out_mtime), Some(in_mtime)) =
            (mtime(&output_apk), newest_input_mtime(project))
        {
            if out_mtime >= in_mtime {
                info!(
                    "I: Built APK is up-to-date (use -f to force rebuild): {}",
                    output_apk.display()
                );
                return Ok(BuildResult {
                    output_apk,
                    entry_count: 0,
                    signed: options.sign.enabled,
                    incremental_skipped: true,
                    used_aapt2: false,
                    used_rust_arsc: false,
                });
            }
        }
    }

    // Prepare text manifest with yml / CLI patches.
    info!("I: patching AndroidManifest.xml");
    let mut manifest_patch = ManifestBuildPatch::new();
    manifest_patch.debuggable = options.debuggable;
    manifest_patch.net_sec_conf = options.net_sec_conf;

    if options.net_sec_conf {
        let xml_dir = project.join("res/xml");
        std::fs::create_dir_all(&xml_dir)?;
        std::fs::write(
            xml_dir.join("network_security_config.xml"),
            network_security_config_xml(),
        )?;
    }

    let text_manifest_path = project.join("AndroidManifest.xml");
    let patched_manifest_path = build_dir.join("AndroidManifest.xml");
    if text_manifest_path.is_file() {
        let raw = std::fs::read_to_string(&text_manifest_path)?;
        let patched = patch_manifest_xml(&raw, &meta, &manifest_patch);
        std::fs::write(&patched_manifest_path, patched)?;
    } else if project.join("original/AndroidManifest.xml").is_file() {
        // Binary-only; copy for packing / copy-original.
        std::fs::copy(
            project.join("original/AndroidManifest.xml"),
            &patched_manifest_path,
        )?;
    }

    // Resource rebuild: original arsc by default. aapt2 / pure-Rust only when requested
    // (`--use-aapt2` / `--rebuild-resources`). Pure-Rust rebuild still fails PackageManager
    // on some real APKs (meta-data / resource resolution).
    let mut used_aapt2 = false;
    let mut used_rust_arsc = false;
    let mut rebuilt_arsc: Option<Vec<u8>> = None;
    let mut aapt2_entries: HashMap<String, Vec<u8>> = HashMap::new();
    let res_dir = project.join("res");
    let can_rebuild = !options.skip_aapt2
        && !options.copy_original
        && res_dir.is_dir()
        && res_dir.join("values/public.xml").is_file();
    let want_rebuild = options.use_aapt2 || options.rebuild_resources;

    if can_rebuild && options.use_aapt2 {
        info!("I: rebuilding resources with aapt2…");
        #[cfg(feature = "aapt2")]
        {
            match try_aapt2_rebuild(project, &meta, options, &patched_manifest_path) {
                Ok(Some(entries)) => {
                    aapt2_entries = entries;
                    used_aapt2 = true;
                    info!("I: aapt2 compile/link succeeded");
                }
                Ok(None) => warn!("W: aapt2 not available; trying pure-Rust ARSC builder"),
                Err(e) => warn!("W: aapt2 failed ({e}); trying pure-Rust ARSC builder"),
            }
        }
        #[cfg(not(feature = "aapt2"))]
        {
            warn!("W: aapt2 feature disabled; trying pure-Rust ARSC builder");
        }
    }

    if can_rebuild && !used_aapt2 && (options.rebuild_resources || options.use_aapt2) {
        info!("I: rebuilding resources.arsc (pure-Rust)…");
        let t_res = Instant::now();
        match build_arsc_from_project(
            project,
            &BuildArscOptions {
                package_id: meta.resourcesInfo.packageId,
                package_name: meta.resourcesInfo.packageName.clone(),
            },
        ) {
            Ok(built) => {
                std::fs::write(build_dir.join("resources.arsc"), &built.arsc)?;
                rebuilt_arsc = Some(built.arsc);
                used_rust_arsc = true;
                info!(
                    "I: pure-Rust ARSC builder wrote {} entries (packageId=0x{:02x}, {:.1}s)",
                    built.entry_count,
                    built.package_id,
                    t_res.elapsed().as_secs_f64()
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
    let dex_dirs = list_dex_dirs(project)?;
    if dex_dirs.is_empty() {
        info!("I: no dex-txt directories; packing root/original DEX as-is");
    } else {
        info!(
            "I: assembling {} DEX file(s) from dex-txt…",
            dex_dirs.len()
        );
    }
    let mut assembled_dex: HashMap<String, Vec<u8>> = HashMap::new();
    for entry in &dex_dirs {
        debug!(
            "I: {} ← {}/",
            entry.apk_dex_name, entry.dir_name
        );
        let dex_bytes = assemble_dex_from_project(project, entry, &assemble_opts)?;
        assembled_dex.insert(entry.apk_dex_name.clone(), dex_bytes);
    }

    info!("I: collecting APK entries…");
    let mut entries = collect_build_entries(project, &meta, &assembled_dex)?;

    // Prefer patched / aapt2 / copy-original manifest.
    entries.retain(|e| e.name != "AndroidManifest.xml" && e.name != "resources.arsc");
    if used_aapt2 {
        entries.retain(|e| !e.name.starts_with("res/"));
        for (name, data) in &aapt2_entries {
            let compress = name != "resources.arsc"
                && !name.ends_with(".png")
                && !name.ends_with(".jpg");
            entries.push(BuildEntry {
                name: name.clone(),
                data: data.clone(),
                compress,
            });
        }
    } else if let Some(arsc) = rebuilt_arsc {
        entries.push(BuildEntry {
            name: "resources.arsc".into(),
            data: arsc,
            compress: false,
        });
    } else {
        let original_arsc = project.join("original/resources.arsc");
        let root_arsc = project.join("resources.arsc");
        let arsc_path = if original_arsc.is_file() {
            original_arsc
        } else {
            root_arsc
        };
        if arsc_path.is_file() {
            entries.push(BuildEntry {
                name: "resources.arsc".into(),
                data: std::fs::read(&arsc_path)?,
                compress: false,
            });
        }
        // Original ARSC references exact ZIP paths (case-sensitive). Prefer the
        // decode-time res-raw.zip so macOS/Windows case collisions cannot drop files.
        if let Some(raw_entries) = load_res_raw_entries(project)? {
            entries.retain(|e| !e.name.starts_with("res/"));
            let n = raw_entries.len();
            entries.extend(raw_entries);
            info!("I: packing {n} res entries from original/res-raw.zip (exact APK case)");
        } else {
            warn!(
                "W: original/res-raw.zip missing; packing decoded res/ \
                 (case-insensitive FS may drop colliding names like res/hq.xml vs res/HQ.xml)"
            );
        }
    }

    if options.copy_original {
        entries.retain(|e| e.name != "AndroidManifest.xml" && !e.name.starts_with("META-INF/"));
        let orig_manifest = project.join("original/AndroidManifest.xml");
        if orig_manifest.is_file() {
            entries.push(BuildEntry {
                name: "AndroidManifest.xml".into(),
                data: std::fs::read(&orig_manifest)?,
                compress: true,
            });
        }
        // Only keep original META-INF when leaving the APK unsigned. A fresh
        // sign must not carry stale JAR digests (they reference old entry set).
        if !options.sign.enabled {
            warn!(
                "W: --copy-original with --no-sign keeps original META-INF; \
                 install will fail until the APK is re-signed"
            );
            let meta_inf = project.join("original/META-INF");
            if meta_inf.is_dir() {
                for entry in WalkDir::new(&meta_inf).into_iter().filter_map(|e| e.ok()) {
                    if !entry.file_type().is_file() {
                        continue;
                    }
                    let rel = entry
                        .path()
                        .strip_prefix(&meta_inf)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/");
                    let name = format!("META-INF/{rel}");
                    entries.push(BuildEntry {
                        name,
                        data: std::fs::read(entry.path())?,
                        compress: true,
                    });
                }
            }
        }
    } else if !used_aapt2 && patched_manifest_path.is_file() {
        // Text (or original binary) patched manifest — encoded later when packing.
        entries.retain(|e| e.name != "AndroidManifest.xml");
        let data = std::fs::read(&patched_manifest_path)?;
        entries.push(BuildEntry {
            name: "AndroidManifest.xml".into(),
            data,
            compress: true,
        });
    }
    // When used_aapt2, linked binary AndroidManifest.xml is already in entries.

    entries.sort_by(|a, b| a.name.cmp(&b.name));
    // Dedup by name (last wins).
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

    // Materialize build/apk/ intermediates.
    info!(
        "I: writing {} intermediates to {}…",
        entries.len(),
        build_apk_dir.display()
    );
    std::fs::create_dir_all(&build_apk_dir)?;
    for entry in &entries {
        let dest = build_apk_dir.join(&entry.name);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let data = encode_pack_entry(entry, options.copy_original)?;
        std::fs::write(&dest, &data)?;
    }

    if options.no_apk {
        info!(
            "I: --no-apk: wrote {} entries to {} ({:.1}s)",
            entries.len(),
            build_apk_dir.display(),
            t_build.elapsed().as_secs_f64()
        );
        return Ok(BuildResult {
            output_apk: build_apk_dir,
            entry_count: entries.len(),
            signed: false,
            incremental_skipped: false,
            used_aapt2,
            used_rust_arsc,
        });
    }

    info!("I: packing ZIP ({} entries)…", entries.len());
    let t_zip = Instant::now();
    let mut writer = ApkWriter::new();
    for entry in &entries {
        let data = encode_pack_entry(entry, options.copy_original)?;
        writer.add_entry(&entry.name, &data, entry.compress);
    }
    let unsigned = writer.finish()?;
    debug!(
        "I: unsigned APK {} bytes ({:.1}s)",
        unsigned.len(),
        t_zip.elapsed().as_secs_f64()
    );

    if options.sign.enabled {
        info!(
            "I: signing APK (v1={}, v2={}, v3={})…",
            options.sign.v1, options.sign.v2, options.sign.v3
        );
    } else {
        warn!("W: --no-sign: APK will not install on device (no certificates)");
        info!("I: skipping sign (--no-sign)");
    }
    let signed_bytes = sign_build_output(&unsigned, &options.sign)?;

    let final_bytes = if meta.packageFormat.is_split_container() {
        crate::container::pack_split_container(project, &meta, &signed_bytes)?
    } else {
        signed_bytes
    };

    std::fs::create_dir_all(&dist_dir)?;
    if let Some(parent) = output_apk.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&output_apk, &final_bytes)?;
    info!(
        "I: wrote {} ({} bytes, {:.1}s total)",
        output_apk.display(),
        final_bytes.len(),
        t_build.elapsed().as_secs_f64()
    );

    Ok(BuildResult {
        output_apk,
        entry_count: entries.len(),
        signed: options.sign.enabled,
        incremental_skipped: false,
        used_aapt2,
        used_rust_arsc,
    })
}

fn load_res_raw_entries(project: &Path) -> Result<Option<Vec<BuildEntry>>> {
    let path = project.join("original/res-raw.zip");
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = std::fs::read(&path)?;
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
    // Text resource XML (layouts/menus/drawables) → binary AXML.
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

#[cfg(feature = "aapt2")]
fn try_aapt2_rebuild(
    project: &Path,
    meta: &ApkToolMeta,
    options: &BuildOptions,
    manifest_path: &Path,
) -> Result<Option<HashMap<String, Vec<u8>>>> {
    let Some(aapt2) = find_aapt2(options.aapt.as_deref()) else {
        return Ok(None);
    };
    if !manifest_path.is_file() {
        return Ok(None);
    }
    // aapt2 link needs text XML manifest.
    let manifest_bytes = std::fs::read(manifest_path)?;
    if looks_binary_axml(&manifest_bytes) {
        return Ok(None);
    }

    let build_dir = project.join("build");
    let compiled = build_dir.join("resources.zip");
    let linked_apk = build_dir.join("resources-linked.apk");
    let res_dir = project.join("res");

    aapt2_compile(
        &aapt2,
        &res_dir,
        &compiled,
        &Aapt2CompileOptions {
            no_crunch: options.no_crunch,
        },
    )
    .map_err(|e| BuildError::Build(e.to_string()))?;

    let mut include = Vec::new();
    let fw_opts = FrameworkOptions {
        frame_path: options.frame_path.clone(),
        tag: options.frame_tag.clone().or_else(|| meta.usesFramework.tag.clone()),
        all_tags: false,
    };
    let ids = if meta.usesFramework.ids.is_empty() {
        vec![1]
    } else {
        meta.usesFramework.ids.clone()
    };
    for id in ids {
        match get_framework_apk(id, &fw_opts) {
            Ok(path) => include.push(path),
            Err(e) => warn!("W: framework id={id} unavailable: {e}"),
        }
    }
    if include.is_empty() {
        if let Some(jar) = find_android_jar(
            meta.sdkInfo
                .targetSdkVersion
                .as_deref()
                .and_then(|s| s.parse().ok()),
        ) {
            include.push(jar);
        }
    }
    if include.is_empty() {
        return Err(BuildError::Build(
            "aapt2 link needs a framework APK (-I) or android.jar".into(),
        ));
    }

    aapt2_link(
        &aapt2,
        &compiled,
        manifest_path,
        &linked_apk,
        &Aapt2LinkOptions {
            include,
            min_sdk: meta.sdkInfo.minSdkVersion.clone(),
            target_sdk: meta.sdkInfo.targetSdkVersion.clone(),
            version_code: meta.versionInfo.versionCode,
            version_name: meta.versionInfo.versionName.clone(),
            package_id: meta.resourcesInfo.packageId,
            replace_version: true,
            output_to_dir: false,
        },
    )
    .map_err(|e| BuildError::Build(e.to_string()))?;

    let linked_bytes = std::fs::read(&linked_apk)?;
    let apk = apkparser::Apk::from_bytes(
        &linked_bytes,
        apkparser::ApkOptions::default().with_signature(false),
    )?;
    let mut out = HashMap::new();
    for name in apk.get_files() {
        if name.ends_with('/') {
            continue;
        }
        if let Ok(data) = apk.get_file(name) {
            out.insert(name.clone(), data);
        }
    }
    Ok(Some(out))
}

#[cfg(feature = "aapt2")]
fn looks_binary_axml(data: &[u8]) -> bool {
    data.len() >= 2 && u16::from_le_bytes([data[0], data[1]]) == 0x0003
}

fn mtime(path: &Path) -> std::io::Result<SystemTime> {
    Ok(std::fs::metadata(path)?.modified()?)
}

fn newest_input_mtime(project: &Path) -> Option<SystemTime> {
    let mut newest: Option<SystemTime> = None;
    let watch = [
        "apkpatch.yml",
        "apktool.yml",
        "AndroidManifest.xml",
        "res",
        "dex",
        "assets",
        "lib",
        "unknown",
        "original",
        "container",
    ];
    for name in watch {
        let path = project.join(name);
        if !path.exists() {
            continue;
        }
        for entry in WalkDir::new(&path).into_iter().filter_map(|e| e.ok()) {
            if !entry.file_type().is_file() {
                continue;
            }
            if let Ok(t) = mtime(entry.path()) {
                newest = Some(match newest {
                    Some(n) => n.max(t),
                    None => t,
                });
            }
        }
        if path.is_file() {
            if let Ok(t) = mtime(&path) {
                newest = Some(match newest {
                    Some(n) => n.max(t),
                    None => t,
                });
            }
        }
    }
    // Also raw root dex files
    if let Ok(rd) = std::fs::read_dir(project) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.ends_with(".dex") {
                if let Ok(t) = mtime(&entry.path()) {
                    newest = Some(match newest {
                        Some(n) => n.max(t),
                        None => t,
                    });
                }
            }
        }
    }
    newest
}
