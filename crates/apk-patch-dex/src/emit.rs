//! Emit dex-txt from a DEX file.

use std::path::{Path, PathBuf};

use dex_bytecode::decode_all_with_resolver;
use dex_parser::{ClassDef, DexFile, NO_INDEX};

use crate::access::{format_access, format_proto};
use crate::jobs::with_jobs;
use crate::naming::class_descriptor_to_path;
use crate::resolver::DexResolver;
use crate::{DexError, Result};

#[derive(Debug, Clone)]
pub struct EmitOptions {
    pub include_debug: bool,
    pub jobs: usize,
}

impl Default for EmitOptions {
    fn default() -> Self {
        Self {
            include_debug: true,
            jobs: 1,
        }
    }
}

struct EmitJob {
    rel: PathBuf,
    content: String,
}

pub fn emit_dex_to_dir(
    dex_bytes: &[u8],
    source_name: &str,
    output_dir: &Path,
    options: &EmitOptions,
) -> Result<usize> {
    if crate::is_odex_bytes(dex_bytes) {
        return Err(DexError::Txt(
            "Cannot disassemble an odex file without deodexing".into(),
        ));
    }
    let dex = DexFile::parse(dex_bytes)?;
    let resolver = DexResolver::new(&dex);

    let class_defs: Vec<ClassDef> = dex.class_defs().flatten().collect();
    let source = source_name.to_string();
    let include_debug = options.include_debug;

    let jobs: Vec<EmitJob> = with_jobs(options.jobs, || {
        #[cfg(feature = "parallel")]
        {
            use rayon::prelude::*;
            class_defs
                .par_iter()
                .map(|class_def| {
                    let content = emit_class(&dex, class_def, &source, &resolver, include_debug)?;
                    let class_name = dex.get_type(class_def.class_idx)?;
                    Ok(EmitJob {
                        rel: class_descriptor_to_path(&class_name),
                        content,
                    })
                })
                .collect::<Result<Vec<_>>>()
        }
        #[cfg(not(feature = "parallel"))]
        {
            class_defs
                .iter()
                .map(|class_def| {
                    let content = emit_class(&dex, class_def, &source, &resolver, include_debug)?;
                    let class_name = dex.get_type(class_def.class_idx)?;
                    Ok(EmitJob {
                        rel: class_descriptor_to_path(&class_name),
                        content,
                    })
                })
                .collect::<Result<Vec<_>>>()
        }
    })?;

    for job in &jobs {
        let out_path = output_dir.join(&job.rel);
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(out_path, &job.content)?;
    }
    Ok(jobs.len())
}

fn emit_class(
    dex: &DexFile,
    class_def: &ClassDef,
    source_name: &str,
    resolver: &DexResolver,
    include_debug: bool,
) -> Result<String> {
    let class_name = dex.get_type(class_def.class_idx)?;
    let super_class = if class_def.superclass_idx != NO_INDEX {
        Some(dex.get_type(class_def.superclass_idx)?)
    } else {
        None
    };
    let source_file = if class_def.source_file_idx != NO_INDEX {
        Some(dex.get_string(class_def.source_file_idx)?)
    } else {
        None
    };

    let mut out = String::new();
    out.push_str("# dex-txt\n");
    out.push_str(&format!("# source: {source_name}\n"));
    out.push_str(&format!("# class: {class_name}\n\n"));
    out.push_str(&format!(
        ".class {} {}\n",
        format_access(class_def.access_flags),
        class_name
    ));
    if let Some(super_class) = super_class {
        out.push_str(&format!(".super {super_class}\n"));
    }
    if let Some(source_file) = source_file {
        if include_debug {
            out.push_str(&format!(".source {source_file}\n"));
        }
    }
    out.push('\n');

    let class_data = match dex.get_class_data(class_def)? {
        Some(data) => data,
        None => return Ok(out),
    };

    for field in &class_data.static_fields {
        let info = dex.get_field_info(field.field_idx)?;
        out.push_str(&format!(
            ".field {} {}:{}\n.end field\n\n",
            format_access(field.access_flags),
            info.name,
            info.typ
        ));
    }
    for field in &class_data.instance_fields {
        let info = dex.get_field_info(field.field_idx)?;
        out.push_str(&format!(
            ".field {} {}:{}\n.end field\n\n",
            format_access(field.access_flags),
            info.name,
            info.typ
        ));
    }

    for method in class_data
        .direct_methods
        .iter()
        .chain(&class_data.virtual_methods)
    {
        emit_method(
            dex,
            method.method_idx,
            method.access_flags,
            method.code_off,
            &mut out,
            resolver,
        )?;
    }

    Ok(out)
}

fn emit_method(
    dex: &DexFile,
    method_idx: u32,
    access_flags: u32,
    code_off: u32,
    out: &mut String,
    resolver: &DexResolver,
) -> Result<()> {
    let info = dex.get_method_info(method_idx)?;
    let proto = format_proto(&info.params, &info.return_type);
    out.push_str(&format!(
        ".method {} {}{}\n",
        format_access(access_flags),
        info.name,
        proto
    ));

    if code_off == 0 {
        out.push_str(".end method\n\n");
        return Ok(());
    }

    let code = dex.get_code_item(code_off)?;
    out.push_str(&format!("    .registers {}\n", code.registers_size));
    out.push_str("    .code\n");

    let insns = code.insns_slice(&dex.data);
    let instructions = decode_all_with_resolver(insns, 0, resolver).map_err(DexError::Bytecode)?;
    for ins in instructions {
        let start = ins.offset as usize;
        let end = start + ins.length as usize;
        let hex: String = insns[start..end]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        out.push_str(&format!(
            "    {:08x}: {:12}  {}\n",
            ins.offset,
            hex,
            sanitize_disasm_annotation(&ins.disasm_line())
        ));
    }

    out.push_str("    .end code\n");
    out.push_str(".end method\n\n");
    Ok(())
}

/// Keep disasm annotations on one line so multi-line strings / binary junk
/// cannot break dex-txt parsing (URLs with `:`, embedded newlines, …).
fn sanitize_disasm_annotation(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}
