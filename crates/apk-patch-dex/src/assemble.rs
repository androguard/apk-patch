//! Assemble dex-txt back into a DEX file by patching the original.

use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use dex_parser::{fix_checksums, replace_code_insns, DexFile};
use log::{debug, info};
use walkdir::WalkDir;

use crate::access::format_proto;
use crate::insn_merge::merge_method_insns;
use crate::jobs::with_jobs;
use crate::naming::{path_to_class_descriptor, DexDirEntry};
use crate::parse::{parse_class_path, DexTxtMethod};
use crate::{DexError, Result};

#[derive(Debug, Clone, Default)]
pub struct AssembleOptions {
    pub jobs: usize,
}

struct MethodPatch {
    code_off: u32,
    insns_hex: Vec<(u32, Vec<u8>)>,
}

pub fn assemble_dex_from_project(
    project: &Path,
    entry: &DexDirEntry,
    options: &AssembleOptions,
) -> Result<Vec<u8>> {
    let original_path = project
        .join("original")
        .join(&entry.apk_dex_name);
    if !original_path.is_file() {
        return Err(DexError::Txt(format!(
            "missing original DEX at {}",
            original_path.display()
        )));
    }
    let original = std::fs::read(&original_path)?;
    assemble_dex_from_txt(&original, &project.join(&entry.dir_name), options)
}

pub fn assemble_dex_from_txt(
    original_dex: &[u8],
    dex_dir: &Path,
    options: &AssembleOptions,
) -> Result<Vec<u8>> {
    let t0 = Instant::now();
    let parsed = DexFile::parse(original_dex)?;
    let method_map = build_method_map(&parsed)?;

    let txt_paths: Vec<_> = WalkDir::new(dex_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_type().is_file()
                && e.path()
                    .extension()
                    .and_then(|x| x.to_str())
                    .is_some_and(|ext| ext == "txt")
        })
        .map(|e| e.into_path())
        .collect();

    let jobs = options.jobs.max(1);
    info!(
        "I: assembling {} ({} class files, jobs={})",
        dex_dir.display(),
        txt_paths.len(),
        jobs
    );

    let method_patches: Vec<MethodPatch> = with_jobs(options.jobs, || {
        #[cfg(feature = "parallel")]
        {
            use rayon::prelude::*;
            txt_paths
                .par_iter()
                .map(|path| collect_patches_from_file(path, &method_map))
                .collect::<Result<Vec<_>>>()
                .map(|nested| nested.into_iter().flatten().collect())
        }
        #[cfg(not(feature = "parallel"))]
        {
            txt_paths
                .iter()
                .map(|path| collect_patches_from_file(path, &method_map))
                .collect::<Result<Vec<_>>>()
                .map(|nested| nested.into_iter().flatten().collect())
        }
    })?;

    debug!(
        "I: parsed {} methods with insn patches from {}",
        method_patches.len(),
        dex_dir.display()
    );

    let mut dex = original_dex.to_vec();
    let mut code_offs: Vec<u32> = method_patches.iter().map(|p| p.code_off).collect();
    code_offs.sort_by(|a, b| b.cmp(a));
    code_offs.dedup();

    // Parse once; only re-parse after a size-changing replace (walk high→low).
    let mut parsed = DexFile::parse(&dex)?;
    let mut replaced = 0usize;
    for code_off in code_offs {
        let code = parsed.get_code_item(code_off)?;
        let original_insns = code.insns_slice(&parsed.data).to_vec();
        let insns_hex: Vec<(u32, Vec<u8>)> = method_patches
            .iter()
            .filter(|p| p.code_off == code_off)
            .flat_map(|p| p.insns_hex.clone())
            .collect();
        let merged = merge_method_insns(&original_insns, &insns_hex)?;
        if merged == original_insns {
            continue;
        }
        replace_code_insns(&mut dex, code_off, &merged)?;
        parsed = DexFile::parse(&dex)?;
        replaced += 1;
    }

    fix_checksums(&mut dex)?;
    info!(
        "I: assembled {} → {} bytes ({} code items changed, {:.1}s)",
        dex_dir.display(),
        dex.len(),
        replaced,
        t0.elapsed().as_secs_f64()
    );
    Ok(dex)
}

fn collect_patches_from_file(
    path: &Path,
    method_map: &HashMap<String, u32>,
) -> Result<Vec<MethodPatch>> {
    let class = parse_class_path(path)?;
    let class_name = if class.class_descriptor.is_empty() {
        path_to_class_descriptor(path)
    } else {
        class.class_descriptor.clone()
    };

    let mut out = Vec::new();
    for method in &class.methods {
        if method.insns_hex.is_empty() {
            continue;
        }
        let key = method_key(&class_name, method);
        let code_off = *method_map
            .get(&key)
            .ok_or_else(|| DexError::Txt(format!("method not found in DEX: {key}")))?;
        out.push(MethodPatch {
            code_off,
            insns_hex: method.insns_hex.clone(),
        });
    }
    Ok(out)
}

fn method_key(class: &str, method: &DexTxtMethod) -> String {
    format!("{}::{}{}", class, method.name, method.proto)
}

fn build_method_map(dex: &DexFile) -> Result<HashMap<String, u32>> {
    let mut map = HashMap::new();
    for class_def in dex.class_defs().flatten() {
        let class_name = dex.get_type(class_def.class_idx)?;
        let Some(class_data) = dex.get_class_data(&class_def)? else {
            continue;
        };
        for enc in class_data
            .direct_methods
            .iter()
            .chain(&class_data.virtual_methods)
        {
            if enc.code_off == 0 {
                continue;
            }
            let info = dex.get_method_info(enc.method_idx)?;
            let proto = format_proto(&info.params, &info.return_type);
            let method = DexTxtMethod {
                access: String::new(),
                name: info.name.clone(),
                proto,
                registers: None,
                insns_hex: Vec::new(),
            };
            let key = method_key(&class_name, &method);
            map.insert(key, enc.code_off);
        }
    }
    Ok(map)
}
