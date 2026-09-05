//! Emit dex-txt from a DEX file.

use std::path::{Path, PathBuf};

use dex_bytecode::{decode_all_with_resolver, format_length, get_opcode_entry, Format};
use dex_parser::{ClassDef, DexFile, NO_INDEX};

use crate::access::{format_access, format_proto};
use crate::jobs::with_jobs;
use crate::naming::class_descriptor_to_path;
use crate::payloads::{format_payload_block, rewrite_payload_targets_to_labels};
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

/// Emit dex-txt class files into a VFS under `output_dir` (logical path).
pub fn emit_dex_to_vfs(
    dex_bytes: &[u8],
    source_name: &str,
    vfs: &mut dyn apk_patch_vfs::Vfs,
    output_dir: &str,
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
        let rel = job.rel.to_string_lossy().replace('\\', "/");
        let out_path = apk_patch_vfs::join_vfs(output_dir, &rel);
        if let Some(parent) = apk_patch_vfs::parent_vfs(&out_path) {
            vfs.create_dir_all(&parent)
                .map_err(|e| DexError::Txt(e.to_string()))?;
        }
        vfs.write(&out_path, job.content.as_bytes())
            .map_err(|e| DexError::Txt(e.to_string()))?;
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
    for iface in dex.get_interfaces(class_def)? {
        out.push_str(&format!(".implements {iface}\n"));
    }
    if let Some(source_file) = source_file {
        if include_debug {
            out.push_str(&format!(".source {source_file}\n"));
        }
    }
    out.push('\n');

    // Class annotations
    let anns = dex.get_annotations(class_def)?;
    for item in &anns.class_annotations {
        emit_annotation(dex, item, &mut out)?;
    }

    let class_data = match dex.get_class_data(class_def)? {
        Some(data) => data,
        None => return Ok(out),
    };

    let static_values = dex.get_static_values(class_def)?;

    for (i, field) in class_data.static_fields.iter().enumerate() {
        let info = dex.get_field_info(field.field_idx)?;
        out.push_str(&format!(
            ".field {} {}:{}\n",
            format_access(field.access_flags),
            info.name,
            info.typ
        ));
        if let Some(v) = static_values.get(i) {
            let lit = format_encoded_value(dex, v);
            out.push_str(&format!("    .value {lit}\n"));
        }
        if let Some((_, set)) = anns
            .field_annotations
            .iter()
            .find(|(idx, _)| *idx == field.field_idx)
        {
            for item in set {
                emit_annotation(dex, item, &mut out)?;
            }
        }
        out.push_str(".end field\n\n");
    }
    for field in &class_data.instance_fields {
        let info = dex.get_field_info(field.field_idx)?;
        out.push_str(&format!(
            ".field {} {}:{}\n",
            format_access(field.access_flags),
            info.name,
            info.typ
        ));
        if let Some((_, set)) = anns
            .field_annotations
            .iter()
            .find(|(idx, _)| *idx == field.field_idx)
        {
            for item in set {
                emit_annotation(dex, item, &mut out)?;
            }
        }
        out.push_str(".end field\n\n");
    }

    for method in class_data
        .direct_methods
        .iter()
        .chain(&class_data.virtual_methods)
    {
        if let Some((_, set)) = anns
            .method_annotations
            .iter()
            .find(|(idx, _)| *idx == method.method_idx)
        {
            for item in set {
                emit_annotation(dex, item, &mut out)?;
            }
        }
        emit_method(
            dex,
            method.method_idx,
            method.access_flags,
            method.code_off,
            &mut out,
            resolver,
            include_debug,
        )?;
    }

    Ok(out)
}

fn emit_annotation(
    dex: &DexFile,
    item: &dex_parser::AnnotationItem,
    out: &mut String,
) -> Result<()> {
    let ty = dex.get_type(item.annotation.type_idx)?;
    out.push_str(&format!(
        ".annotation {} {}\n",
        dex_parser::visibility_name(item.visibility),
        ty
    ));
    for (name_idx, val) in &item.annotation.elements {
        let name = dex.get_string(*name_idx)?;
        emit_annotation_element(dex, &name, val, out, "    ")?;
    }
    out.push_str(".end annotation\n\n");
    Ok(())
}

fn emit_annotation_element(
    dex: &DexFile,
    name: &str,
    val: &dex_parser::EncodedValue,
    out: &mut String,
    indent: &str,
) -> Result<()> {
    if let dex_parser::EncodedValue::Annotation(ann) = val {
        let ty = dex.get_type(ann.type_idx)?;
        out.push_str(&format!("{indent}{name} = .subannotation {ty}\n"));
        let nested = format!("{indent}    ");
        for (name_idx, v) in &ann.elements {
            let n = dex.get_string(*name_idx)?;
            emit_annotation_element(dex, &n, v, out, &nested)?;
        }
        out.push_str(&format!("{indent}.end subannotation\n"));
        return Ok(());
    }
    let lit = format_encoded_value(dex, val);
    out.push_str(&format!("{indent}{name} = {lit}\n"));
    Ok(())
}

fn emit_method(
    dex: &DexFile,
    method_idx: u32,
    access_flags: u32,
    code_off: u32,
    out: &mut String,
    resolver: &DexResolver,
    include_debug: bool,
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

    if include_debug {
        if let Ok(dbg) = dex.debug_info_for_code(&code) {
            for name in &dbg.parameter_names {
                if let Some(n) = name {
                    out.push_str(&format!("    .param \"{n}\"\n"));
                }
            }
            if dbg.line_start > 0 {
                out.push_str(&format!("    .line {}\n", dbg.line_start));
            }
            for (reg, name) in &dbg.register_names {
                out.push_str(&format!("    .local v{reg} \"{name}\" Ljava/lang/Object;\n"));
            }
        }
    }

    let insns = code.insns_slice(&dex.data);
    let instructions = decode_all_with_resolver(insns, 0, resolver).map_err(DexError::Bytecode)?;
    for ins in &instructions {
        let label = format!(":L_{:08x}", ins.offset);
        let hex: String = insns[ins.offset as usize..ins.offset as usize + ins.length as usize]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        out.push_str(&format!("    {label}\n"));
        if ins.mnemonic.contains("payload") {
            let slice = &insns[ins.offset as usize..ins.offset as usize + ins.length as usize];
            match format_payload_block(slice) {
                Ok(block) => {
                    let switch_off = find_payload_ref_offset(&instructions, ins.offset);
                    let valid: std::collections::HashSet<u32> =
                        instructions.iter().map(|i| i.offset).collect();
                    let block = if let Some(soff) = switch_off {
                        rewrite_payload_targets_to_labels(&block, soff, &valid)
                    } else {
                        block
                    };
                    out.push_str(&block);
                }
                Err(_) => {
                    out.push_str(&format!("    .hex {hex}\n"));
                }
            }
        } else {
            let mut line = ins.disasm_line();
            // Quote/escape const-string before sanitize so binary bytes become \uXXXX
            // rather than \xHH sprinkled into an unquoted operand.
            line = quote_string_operands(&ins.mnemonic, &line);
            line = sanitize_disasm_outside_strings(&line);
            line = rewrite_branch_operands_to_labels(&ins.mnemonic, ins.opcode, ins.offset, &line);
            line = rewrite_payload_ref_to_label(&ins.mnemonic, ins.opcode, ins.offset, &line);
            out.push_str(&format!("    {line}  # {hex}\n"));
        }
    }

    // Exclusive catch ends (and rare mid-gap labels) may point past the last
    // instruction; emit bare labels so assemble/verify can resolve them.
    let insn_offs: std::collections::HashSet<u32> =
        instructions.iter().map(|i| i.offset).collect();
    if let Ok(tries) = code.tries(&dex.data) {
        let mut trailing: Vec<u32> = Vec::new();
        for t in &tries {
            for off in [
                t.start_unit * 2,
                (t.start_unit + t.insn_count as u32) * 2,
            ] {
                if !insn_offs.contains(&off) {
                    trailing.push(off);
                }
            }
            for (_, handler) in &t.handlers {
                let off = handler * 2;
                if !insn_offs.contains(&off) {
                    trailing.push(off);
                }
            }
        }
        trailing.sort_unstable();
        trailing.dedup();
        for off in trailing {
            out.push_str(&format!("    :L_{:08x}\n", off));
        }
        for t in tries {
            let start = format!(":L_{:08x}", t.start_unit * 2);
            let end = format!(":L_{:08x}", (t.start_unit + t.insn_count as u32) * 2);
            for (ty, handler) in &t.handlers {
                let handler_l = format!(":L_{:08x}", handler * 2);
                match ty {
                    Some(idx) => {
                        let typ = dex.get_type(*idx)?;
                        out.push_str(&format!(
                            "    .catch {typ} {{ {start} .. {end} }} {handler_l}\n"
                        ));
                    }
                    None => {
                        out.push_str(&format!(
                            "    .catchall {{ {start} .. {end} }} {handler_l}\n"
                        ));
                    }
                }
            }
        }
    }

    out.push_str(".end method\n\n");
    Ok(())
}

fn quote_string_operands(mnemonic: &str, disasm: &str) -> String {
    if !mnemonic.starts_with("const-string") {
        return disasm.to_string();
    }
    // "const-string v0, hello" → quote the string part
    let Some((regs, s)) = disasm.split_once(',') else {
        return disasm.to_string();
    };
    // Decoder formats `vN, {raw}`; keep leading/trailing spaces as string content.
    let s = s.strip_prefix(' ').unwrap_or(s);
    if s.starts_with("string@") {
        return disasm.to_string();
    }
    // Raw pool string may itself start with `"` — always escape, never assume quoted.
    format!(
        "{}, \"{}\"",
        regs.trim_end(),
        dex_parser::escape_string_literal(s)
    )
}

/// Sanitize control chars outside of `"..."` string literals.
fn sanitize_disasm_outside_strings(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_str = false;
    let mut escape = false;
    for ch in s.chars() {
        if in_str {
            out.push(ch);
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_str = false;
            }
            continue;
        }
        if ch == '"' {
            in_str = true;
            out.push(ch);
            continue;
        }
        match ch {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn rewrite_branch_operands_to_labels(
    mnemonic: &str,
    opcode: u8,
    offset: u32,
    disasm: &str,
) -> String {
    let entry = get_opcode_entry(opcode);
    let is_branch = matches!(
        entry.format,
        Format::F10t | Format::F20t | Format::F21t | Format::F22t | Format::F30t | Format::F31t
    );
    // F31t is also used by fill-array-data / packed-switch / sparse-switch (payload refs).
    let is_payload_ref = matches!(
        mnemonic,
        "fill-array-data" | "packed-switch" | "sparse-switch"
    );
    if !is_branch || is_payload_ref {
        return disasm.to_string();
    }
    // Operate on operands only (after mnemonic).
    let (mnem, operands) = disasm
        .split_once(char::is_whitespace)
        .map(|(a, b)| (a, b.trim()))
        .unwrap_or((disasm, ""));
    if operands.is_empty() {
        return disasm.to_string();
    }
    let parts: Vec<&str> = operands.split_whitespace().collect();
    let last = parts[parts.len() - 1].trim_end_matches(',');
    let Some(rel) = parse_rel_units(last) else {
        return disasm.to_string();
    };
    let target_bytes = (offset as i32) + rel * 2;
    if target_bytes < 0 {
        return disasm.to_string();
    }
    let target_label = format!(":L_{:08x}", target_bytes as u32);
    let mut ops_out = String::new();
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            ops_out.push(' ');
        }
        if i == parts.len() - 1 {
            // Preserve trailing comma style without forcing one
            ops_out.push_str(&target_label);
        } else {
            ops_out.push_str(p);
        }
    }
    let _ = format_length;
    format!("{mnem} {ops_out}")
}

fn parse_rel_units(tok: &str) -> Option<i32> {
    let body = tok.trim_end_matches('h');
    let (neg, digits) = if let Some(rest) = body.strip_prefix('+') {
        (false, rest)
    } else if let Some(rest) = body.strip_prefix('-') {
        (true, rest)
    } else {
        (false, body)
    };
    let v = i32::from_str_radix(digits, 16).ok()?;
    Some(if neg { -v } else { v })
}

fn rewrite_payload_ref_to_label(
    mnemonic: &str,
    opcode: u8,
    offset: u32,
    disasm: &str,
) -> String {
    let is_payload_ref = matches!(
        mnemonic,
        "fill-array-data" | "packed-switch" | "sparse-switch"
    );
    if !is_payload_ref {
        return disasm.to_string();
    }
    let entry = get_opcode_entry(opcode);
    if entry.format != Format::F31t {
        return disasm.to_string();
    }
    let (mnem, operands) = disasm
        .split_once(char::is_whitespace)
        .map(|(a, b)| (a, b.trim()))
        .unwrap_or((disasm, ""));
    let parts: Vec<&str> = operands.split_whitespace().collect();
    if parts.is_empty() {
        return disasm.to_string();
    }
    let last = parts[parts.len() - 1].trim_end_matches(',');
    let Some(rel) = parse_rel_units(last) else {
        return disasm.to_string();
    };
    let target_bytes = (offset as i32) + rel * 2;
    if target_bytes < 0 {
        return disasm.to_string();
    }
    let target_label = format!(":L_{:08x}", target_bytes as u32);
    let mut ops_out = String::new();
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            ops_out.push(' ');
        }
        if i == parts.len() - 1 {
            ops_out.push_str(&target_label);
        } else {
            ops_out.push_str(p);
        }
    }
    format!("{mnem} {ops_out}")
}

fn find_payload_ref_offset(
    instructions: &[dex_bytecode::Instruction],
    payload_byte_off: u32,
) -> Option<u32> {
    for ins in instructions {
        if !matches!(
            ins.mnemonic,
            "fill-array-data" | "packed-switch" | "sparse-switch"
        ) {
            continue;
        }
        // F31t: relative units in last 4 bytes of 6-byte insn
        // Use disasm operands via branch: decode relative from format
        let entry = get_opcode_entry(ins.opcode);
        if entry.format != Format::F31t {
            continue;
        }
        // Parse from raw would need bytes; approximate via operand string if present
        let ops = &ins.operands;
        for tok in ops.split_whitespace() {
            if let Some(rel) = parse_rel_units(tok.trim_end_matches(',')) {
                let target = (ins.offset as i32) + rel * 2;
                if target >= 0 && target as u32 == payload_byte_off {
                    return Some(ins.offset);
                }
            }
        }
    }
    None
}

/// Keep disasm annotations on one line so multi-line strings / binary junk
/// cannot break dex-txt parsing (URLs with `:`, embedded newlines, …).
#[allow(dead_code)]
fn sanitize_disasm_annotation(s: &str) -> String {
    sanitize_disasm_outside_strings(s)
}

fn format_encoded_value(dex: &DexFile, v: &dex_parser::EncodedValue) -> String {
    dex_parser::format_value_literal_full(
        v,
        &|idx| dex.get_string(idx).ok(),
        &|idx| dex.get_type(idx).ok(),
        &|idx| {
            dex.get_field_info(idx)
                .ok()
                .map(|f| format!("{}->{}:{}", f.class, f.name, f.typ))
        },
        &|idx| {
            dex.get_method_info(idx).ok().map(|m| {
                let proto = format_proto(&m.params, &m.return_type);
                format!("{}->{}{}", m.class, m.name, proto)
            })
        },
    )
}
