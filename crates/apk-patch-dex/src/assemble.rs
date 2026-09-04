//! Assemble dex-txt into a new DEX via DexBuilder (mnemonic-first).

use std::collections::HashMap;
use std::path::Path;

use dex_bytecode::{encode_instruction, format_length, get_opcode_entry, opcode_for_mnemonic, EncodeResolve};
use dex_parser::{
    build_debug_info, parse_value_literal, parse_visibility, simple_annotation, AnnotationItem,
    AnnotationsDirectory, BuiltClass, BuiltCode, BuiltField, BuiltMethod, BuiltTry,
    DebugBuilderOp, DexBuilder, EncodedValue, PoolMaps,
};
use log::info;
use walkdir::WalkDir;

use crate::access::{parse_access, proto_ins_words};
use crate::jobs::with_jobs;
use crate::naming::DexDirEntry;
use crate::parse::{
    parse_class_file, DexTxtAnnotation, DexTxtCatch, DexTxtClass, DexTxtDebug, DexTxtInsn,
    DexTxtMethod,
};
use crate::payloads::{encode_payload_bytes, payload_size_units};
use crate::verify::verify_class;
use crate::{DexError, Result};

/// `Instant` panics on `wasm32-unknown-unknown`.
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

#[derive(Debug, Clone, Default)]
pub struct AssembleOptions {
    pub jobs: usize,
}

pub fn assemble_dex_from_project(
    project: &Path,
    entry: &DexDirEntry,
    options: &AssembleOptions,
) -> Result<Vec<u8>> {
    assemble_dex_from_txt(&project.join(&entry.dir_name), options)
}

pub fn assemble_dex_from_txt(dex_dir: &Path, options: &AssembleOptions) -> Result<Vec<u8>> {
    let t0 = Tick::now();
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

    let classes: Vec<DexTxtClass> = with_jobs(options.jobs, || {
        txt_paths
            .iter()
            .map(|path| {
                let data = std::fs::read_to_string(path)?;
                parse_class_file(&data)
            })
            .collect::<Result<Vec<_>>>()
    })?;

    let bytes = assemble_classes(&classes)?;
    info!(
        "I: assembled {} → {} bytes ({:.1}s)",
        dex_dir.display(),
        bytes.len(),
        t0.secs_f64()
    );
    Ok(bytes)
}

pub fn assemble_dex_from_vfs(
    vfs: &dyn apk_patch_vfs::Vfs,
    project: &str,
    entry: &DexDirEntry,
    options: &AssembleOptions,
) -> Result<Vec<u8>> {
    let dex_dir = apk_patch_vfs::join_vfs(project, &entry.dir_name);
    assemble_dex_from_txt_vfs(vfs, &dex_dir, options)
}

pub fn assemble_dex_from_txt_vfs(
    vfs: &dyn apk_patch_vfs::Vfs,
    dex_dir: &str,
    _options: &AssembleOptions,
) -> Result<Vec<u8>> {
    let t0 = Tick::now();
    let mut txt_paths = Vec::new();
    collect_txt_vfs(vfs, dex_dir, &mut txt_paths)?;

    info!(
        "I: assembling {} ({} class files)",
        dex_dir,
        txt_paths.len()
    );

    let classes: Vec<DexTxtClass> = txt_paths
        .iter()
        .map(|path| {
            let data = vfs
                .read(path)
                .map_err(|e| DexError::Txt(e.to_string()))?;
            let text = String::from_utf8_lossy(&data);
            parse_class_file(&text)
        })
        .collect::<Result<Vec<_>>>()?;

    let bytes = assemble_classes(&classes)?;
    info!(
        "I: assembled {} → {} bytes ({:.1}s)",
        dex_dir,
        bytes.len(),
        t0.secs_f64()
    );
    Ok(bytes)
}

fn collect_txt_vfs(
    vfs: &dyn apk_patch_vfs::Vfs,
    dir: &str,
    out: &mut Vec<String>,
) -> Result<()> {
    let entries = vfs
        .read_dir(dir)
        .map_err(|e| DexError::Txt(e.to_string()))?;
    for entry in entries {
        let path = apk_patch_vfs::join_vfs(dir, &entry.name);
        if vfs.is_dir(&path) {
            collect_txt_vfs(vfs, &path, out)?;
        } else if entry.name.ends_with(".txt") {
            out.push(path);
        }
    }
    Ok(())
}

fn assemble_classes(classes: &[DexTxtClass]) -> Result<Vec<u8>> {
    for class in classes {
        verify_class(class)?;
    }

    let mut builder = DexBuilder::new();

    // First pass: intern declarations + operand refs
    for class in classes {
        intern_class_refs(&mut builder, class)?;
    }

    let maps = builder.pool_maps();
    let resolve = MapsResolve { maps: &maps };

    for class in classes {
        let built = build_class(class, &resolve, &maps)?;
        builder.add_class(built);
    }

    builder
        .finish()
        .map_err(|e| DexError::Txt(e.to_string()))
}

fn intern_class_refs(builder: &mut DexBuilder, class: &DexTxtClass) -> Result<()> {
    builder.intern_type(&class.class_descriptor);
    if let Some(ref s) = class.super_class {
        builder.intern_type(s);
    }
    for iface in &class.interfaces {
        builder.intern_type(iface);
    }
    if let Some(ref sf) = class.source_file {
        builder.intern_string(sf);
    }
    for ann in &class.annotations {
        intern_annotation(builder, ann)?;
    }
    for f in &class.fields {
        builder.intern_field_ref(&class.class_descriptor, &f.name, &f.typ);
        if let Some(ref v) = f.value {
            intern_value_literal(builder, v)?;
        }
        for ann in &f.annotations {
            intern_annotation(builder, ann)?;
        }
    }
    for m in &class.methods {
        builder.intern_method_ref(&class.class_descriptor, &m.name, &m.proto);
        for ann in &m.annotations {
            intern_annotation(builder, ann)?;
        }
        for p in &m.params {
            builder.intern_string(p);
        }
        for insn in &m.insns {
            intern_operand_refs(builder, &insn.operands)?;
        }
        for c in &m.catches {
            if let Some(ref ty) = c.exception_type {
                builder.intern_type(ty);
            }
        }
        for d in &m.debug {
            if let DexTxtDebug::Local { name, typ, .. } = d {
                builder.intern_string(name);
                builder.intern_type(typ);
            }
        }
    }
    Ok(())
}

fn intern_annotation(builder: &mut DexBuilder, ann: &DexTxtAnnotation) -> Result<()> {
    builder.intern_type(&ann.typ);
    for (name, val) in &ann.elements {
        builder.intern_string(name);
        intern_value_literal(builder, val)?;
    }
    Ok(())
}

fn intern_value_literal(builder: &mut DexBuilder, lit: &str) -> Result<()> {
    let lit = lit.trim();
    let lit = lit.strip_prefix(".enum ").unwrap_or(lit).trim();
    if lit.starts_with('"') && lit.ends_with('"') && lit.len() >= 2 {
        builder.intern_string(&lit[1..lit.len() - 1]);
    } else if lit.starts_with('L') && lit.ends_with(';') && !lit.contains("->") {
        builder.intern_type(lit);
    } else if let Some((class, rest)) = lit.split_once("->") {
        builder.intern_type(class);
        if rest.contains('(') {
            if let Some((name, proto)) = rest.split_once('(') {
                let proto = format!("({proto}");
                builder.intern_method_ref(class, name, &proto);
            }
        } else if let Some((name, typ)) = rest.split_once(':') {
            builder.intern_field_ref(class, name, typ);
            builder.intern_type(typ);
        }
    } else if lit.starts_with('{') && lit.ends_with('}') {
        let inner = &lit[1..lit.len() - 1];
        for part in inner.split(',') {
            intern_value_literal(builder, part.trim())?;
        }
    } else if lit.starts_with('[') {
        builder.intern_type(lit);
    }
    Ok(())
}

fn intern_operand_refs(builder: &mut DexBuilder, operands: &str) -> Result<()> {
    for tok in tokenize_refs(operands) {
        if tok.starts_with('"') {
            let inner = tok.trim_matches('"');
            builder.intern_string(inner);
        } else if let Some((class, rest)) = tok.split_once("->") {
            // Method or field ref on any type descriptor (L...; or [J or [L...;)
            if rest.contains('(') {
                if let Some((name, proto)) = rest.split_once('(') {
                    let proto = format!("({proto}");
                    builder.intern_method_ref(class, name, &proto);
                }
            } else if let Some((name, typ)) = rest.split_once(':') {
                builder.intern_field_ref(class, name, typ);
            }
        } else if tok.starts_with('L') && tok.ends_with(';') {
            builder.intern_type(tok);
        } else if tok.starts_with('(') {
            builder.intern_proto(tok);
        } else if tok.starts_with('[') {
            builder.intern_type(tok);
        } else if matches!(
            tok.chars().next(),
            Some('Z' | 'B' | 'S' | 'C' | 'I' | 'J' | 'F' | 'D' | 'V')
        ) && tok.len() == 1
        {
            builder.intern_type(tok);
        }
    }
    Ok(())
}

fn tokenize_refs(operands: &str) -> Vec<&str> {
    // Split on commas outside quotes; keep method/field tokens intact.
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut in_str = false;
    let bytes = operands.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'"' {
            in_str = !in_str;
        } else if b == b',' && !in_str {
            let tok = operands[start..i].trim();
            if !tok.is_empty() {
                out.push(tok);
            }
            start = i + 1;
        }
    }
    let tok = operands[start..].trim();
    if !tok.is_empty() {
        out.push(tok);
    }
    out
}

struct MapsResolve<'a> {
    maps: &'a PoolMaps,
}

impl EncodeResolve for MapsResolve<'_> {
    fn resolve_string(&self, s: &str) -> std::result::Result<u32, dex_bytecode::DexError> {
        self.maps
            .string_idx
            .get(s)
            .copied()
            .ok_or_else(|| dex_bytecode::DexError::invalid_owned(format!("string not in pool: {s}")))
    }
    fn resolve_type(&self, s: &str) -> std::result::Result<u32, dex_bytecode::DexError> {
        self.maps
            .type_idx
            .get(s)
            .copied()
            .ok_or_else(|| dex_bytecode::DexError::invalid_owned(format!("type not in pool: {s}")))
    }
    fn resolve_field(&self, s: &str) -> std::result::Result<u32, dex_bytecode::DexError> {
        let (c, n, t) = parse_field_ref(s).map_err(dex_bytecode::DexError::invalid_owned)?;
        self.maps
            .field_idx
            .get(&(c, n, t))
            .copied()
            .ok_or_else(|| dex_bytecode::DexError::invalid_owned(format!("field not in pool: {s}")))
    }
    fn resolve_method(&self, s: &str) -> std::result::Result<u32, dex_bytecode::DexError> {
        let (c, n, p) = parse_method_ref(s).map_err(dex_bytecode::DexError::invalid_owned)?;
        self.maps
            .method_idx
            .get(&(c.clone(), n.clone(), p.clone()))
            .copied()
            .ok_or_else(|| dex_bytecode::DexError::invalid_owned(format!("method not in pool: {s}")))
    }
    fn resolve_proto(&self, s: &str) -> std::result::Result<u32, dex_bytecode::DexError> {
        self.maps
            .proto_idx
            .get(s)
            .copied()
            .ok_or_else(|| dex_bytecode::DexError::invalid_owned(format!("proto not in pool: {s}")))
    }
}

fn parse_field_ref(s: &str) -> std::result::Result<(String, String, String), String> {
    let (class, rest) = s
        .split_once("->")
        .ok_or_else(|| format!("bad field ref: {s}"))?;
    let (name, typ) = rest
        .split_once(':')
        .ok_or_else(|| format!("bad field ref: {s}"))?;
    Ok((class.to_string(), name.to_string(), typ.to_string()))
}

fn parse_method_ref(s: &str) -> std::result::Result<(String, String, String), String> {
    let (class, rest) = s
        .split_once("->")
        .ok_or_else(|| format!("bad method ref: {s}"))?;
    let (name, proto) = rest
        .split_once('(')
        .ok_or_else(|| format!("bad method ref: {s}"))?;
    Ok((class.to_string(), name.to_string(), format!("({proto}")))
}

fn build_class(class: &DexTxtClass, resolve: &MapsResolve, maps: &PoolMaps) -> Result<BuiltClass> {
    let access = parse_access(&class.access);
    let mut static_fields = Vec::new();
    let mut instance_fields = Vec::new();
    for f in &class.fields {
        let af = parse_access(&f.access);
        let static_value = if af & 0x0008 != 0 {
            f.value
                .as_ref()
                .map(|lit| resolve_value_lit(lit, maps))
                .transpose()?
        } else {
            None
        };
        let bf = BuiltField {
            class: class.class_descriptor.clone(),
            name: f.name.clone(),
            typ: f.typ.clone(),
            access_flags: af,
            static_field: af & 0x0008 != 0,
            static_value,
        };
        if bf.static_field {
            static_fields.push(bf);
        } else {
            instance_fields.push(bf);
        }
    }

    let mut direct_methods = Vec::new();
    let mut virtual_methods = Vec::new();
    for m in &class.methods {
        let af = parse_access(&m.access);
        let direct = is_direct_method(af, &m.name);
        let code = if m.insns.is_empty() && m.registers.is_none() {
            None
        } else {
            Some(assemble_method_code(m, af, resolve, maps)?)
        };
        let bm = BuiltMethod {
            class: class.class_descriptor.clone(),
            name: m.name.clone(),
            proto: m.proto.clone(),
            access_flags: af,
            code,
            direct,
        };
        if direct {
            direct_methods.push(bm);
        } else {
            virtual_methods.push(bm);
        }
    }

    let annotations = build_annotations_dir(class, maps)?;

    Ok(BuiltClass {
        descriptor: class.class_descriptor.clone(),
        access_flags: access,
        superclass: class.super_class.clone(),
        interfaces: class.interfaces.clone(),
        source_file: class.source_file.clone(),
        static_fields,
        instance_fields,
        direct_methods,
        virtual_methods,
        annotations,
    })
}

fn resolve_value_lit(lit: &str, maps: &PoolMaps) -> Result<EncodedValue> {
    let lit = lit.trim();
    if lit.starts_with("method@")
        || lit.starts_with("field@")
        || lit.starts_with("enum@")
        || lit.starts_with("method_type@")
        || lit.starts_with("method_handle@")
    {
        return Err(DexError::Txt(format!(
            "pool-index .value {lit} is not rebuildable; use symbolic field/method refs"
        )));
    }
    if let Some(rest) = lit.strip_prefix(".enum ") {
        let (c, n, t) = parse_field_ref(rest.trim()).map_err(DexError::Txt)?;
        let idx = maps
            .field_idx
            .get(&(c, n, t))
            .copied()
            .ok_or_else(|| DexError::Txt(format!("enum field not in pool: {rest}")))?;
        return Ok(EncodedValue::Enum(idx));
    }
    if lit.contains("->") {
        if lit.contains('(') {
            let (c, n, p) = parse_method_ref(lit).map_err(DexError::Txt)?;
            let idx = maps
                .method_idx
                .get(&(c, n, p))
                .copied()
                .ok_or_else(|| DexError::Txt(format!("method value not in pool: {lit}")))?;
            return Ok(EncodedValue::Method(idx));
        }
        if lit.contains(':') {
            let (c, n, t) = parse_field_ref(lit).map_err(DexError::Txt)?;
            let idx = maps
                .field_idx
                .get(&(c, n, t))
                .copied()
                .ok_or_else(|| DexError::Txt(format!("field value not in pool: {lit}")))?;
            return Ok(EncodedValue::Field(idx));
        }
    }
    use std::cell::RefCell;
    let err: RefCell<Option<String>> = RefCell::new(None);
    let v = parse_value_literal(
        lit,
        &mut |s| match maps.string_idx.get(s) {
            Some(&i) => i,
            None => {
                *err.borrow_mut() = Some(format!("string not interned: {s}"));
                0
            }
        },
        &mut |t| match maps.type_idx.get(t) {
            Some(&i) => i,
            None => {
                *err.borrow_mut() = Some(format!("type not interned: {t}"));
                0
            }
        },
    )
    .map_err(|e| DexError::Txt(e.to_string()))?;
    if let Some(e) = err.into_inner() {
        return Err(DexError::Txt(e));
    }
    Ok(v)
}

fn build_txt_annotation(ann: &DexTxtAnnotation, maps: &PoolMaps) -> Result<AnnotationItem> {
    let type_idx = maps
        .type_idx
        .get(&ann.typ)
        .copied()
        .ok_or_else(|| DexError::Txt(format!("annotation type not in pool: {}", ann.typ)))?;
    let mut elements = Vec::new();
    for (name, val) in &ann.elements {
        let name_idx = maps
            .string_idx
            .get(name)
            .copied()
            .ok_or_else(|| DexError::Txt(format!("annotation name not in pool: {name}")))?;
        elements.push((name_idx, resolve_value_lit(val, maps)?));
    }
    Ok(simple_annotation(
        type_idx,
        elements,
        parse_visibility(&ann.visibility),
    ))
}

fn build_annotations_dir(
    class: &DexTxtClass,
    maps: &PoolMaps,
) -> Result<AnnotationsDirectory> {
    let mut dir = AnnotationsDirectory::default();
    for ann in &class.annotations {
        dir.class_annotations.push(build_txt_annotation(ann, maps)?);
    }
    for f in &class.fields {
        if f.annotations.is_empty() {
            continue;
        }
        let idx = maps
            .field_idx
            .get(&(
                class.class_descriptor.clone(),
                f.name.clone(),
                f.typ.clone(),
            ))
            .copied()
            .ok_or_else(|| DexError::Txt(format!("field not in pool: {}->{}:{}", class.class_descriptor, f.name, f.typ)))?;
        let set = f
            .annotations
            .iter()
            .map(|a| build_txt_annotation(a, maps))
            .collect::<Result<Vec<_>>>()?;
        dir.field_annotations.push((idx, set));
    }
    for m in &class.methods {
        if m.annotations.is_empty() {
            continue;
        }
        let idx = maps
            .method_idx
            .get(&(
                class.class_descriptor.clone(),
                m.name.clone(),
                m.proto.clone(),
            ))
            .copied()
            .ok_or_else(|| {
                DexError::Txt(format!(
                    "method not in pool: {}->{}{}",
                    class.class_descriptor, m.name, m.proto
                ))
            })?;
        let set = m
            .annotations
            .iter()
            .map(|a| build_txt_annotation(a, maps))
            .collect::<Result<Vec<_>>>()?;
        dir.method_annotations.push((idx, set));
    }
    Ok(dir)
}

fn is_direct_method(access: u32, name: &str) -> bool {
    // static, private, or constructor
    access & 0x0008 != 0 || access & 0x0002 != 0 || access & 0x10000 != 0 || name.starts_with('<')
}

fn assemble_method_code(
    method: &DexTxtMethod,
    access: u32,
    resolve: &MapsResolve,
    maps: &PoolMaps,
) -> Result<BuiltCode> {
    let is_static = access & 0x0008 != 0;
    let ins_size = proto_ins_words(&method.proto, is_static);
    let registers = method.registers.unwrap_or(ins_size.max(1));

    // Rewrite pN → v(registers - ins_size + N)
    let rewritten: Vec<DexTxtInsn> = method
        .insns
        .iter()
        .map(|insn| DexTxtInsn {
            label: insn.label.clone(),
            mnemonic: insn.mnemonic.clone(),
            operands: rewrite_p_regs(&insn.operands, registers, ins_size),
        })
        .collect();

    let (insns, label_units) = encode_insns(&rewritten, resolve)?;
    let tries = build_tries(&method.catches, &label_units, maps)?;
    let outs_size = estimate_outs(&rewritten);
    let debug_ops = build_debug_ops(method, maps);

    Ok(BuiltCode {
        registers_size: registers,
        ins_size,
        outs_size,
        insns,
        tries,
        debug_info_off: 0,
        debug_ops,
    })
}

fn rewrite_p_regs(operands: &str, registers: u16, ins_size: u16) -> String {
    let base = registers.saturating_sub(ins_size);
    let mut out = String::with_capacity(operands.len());
    let mut chars = operands.chars().peekable();
    while let Some(c) = chars.next() {
        if c == 'p' {
            let mut num = String::new();
            while let Some(d) = chars.peek().copied().filter(|d| d.is_ascii_digit()) {
                num.push(d);
                chars.next();
            }
            if !num.is_empty() {
                if let Ok(n) = num.parse::<u16>() {
                    out.push('v');
                    out.push_str(&(base + n).to_string());
                    continue;
                }
            }
            out.push('p');
            out.push_str(&num);
        } else {
            out.push(c);
        }
    }
    out
}

fn encode_insns(
    insns: &[DexTxtInsn],
    resolve: &MapsResolve,
) -> Result<(Vec<u8>, HashMap<String, u32>)> {
    // Pass 1: sizes and label → code unit offset
    let mut label_units: HashMap<String, u32> = HashMap::new();
    let mut unit = 0u32;
    let mut sizes = Vec::with_capacity(insns.len());
    for insn in insns {
        if let Some(ref lab) = insn.label {
            label_units.insert(lab.clone(), unit);
        }
        let units = if insn.mnemonic.starts_with(".hex") {
            let bytes = hex_decode(&insn.operands)?;
            (bytes.len() as u32) / 2
        } else if matches!(
            insn.mnemonic.as_str(),
            ".array-data" | ".packed-switch" | ".sparse-switch"
        ) {
            payload_size_units(&insn.mnemonic, &insn.operands)?
        } else {
            let op = opcode_for_mnemonic(&insn.mnemonic).ok_or_else(|| {
                DexError::Txt(format!("unknown mnemonic: {}", insn.mnemonic))
            })?;
            let entry = get_opcode_entry(op);
            format_length(entry.format) / 2
        };
        sizes.push(units);
        unit += units;
    }

    // Map payload label → switch insn unit (for relative targets)
    let switch_base_for_payload = find_switch_bases(insns, &label_units);

    // Pass 2: encode with branch resolution
    let mut out = Vec::new();
    let mut here = 0u32;
    for (i, insn) in insns.iter().enumerate() {
        if insn.mnemonic.starts_with(".hex") {
            out.extend(hex_decode(&insn.operands)?);
        } else if matches!(
            insn.mnemonic.as_str(),
            ".array-data" | ".packed-switch" | ".sparse-switch"
        ) {
            let base = insn
                .label
                .as_ref()
                .and_then(|l| switch_base_for_payload.get(l).copied())
                .unwrap_or(here);
            out.extend(encode_payload_bytes(
                &insn.mnemonic,
                &insn.operands,
                base,
                &label_units,
            )?);
        } else {
            let branch_rel = branch_rel_for(insn, here, &label_units);
            let bytes = encode_instruction(&insn.mnemonic, &insn.operands, resolve, branch_rel)
                .map_err(|e| DexError::Txt(format!("encode {}: {e}", insn.mnemonic)))?;
            out.extend_from_slice(&bytes);
        }
        here += sizes[i];
    }
    Ok((out, label_units))
}

fn find_switch_bases(
    insns: &[DexTxtInsn],
    labels: &HashMap<String, u32>,
) -> HashMap<String, u32> {
    let mut map = HashMap::new();
    let mut unit = 0u32;
    for insn in insns {
        if matches!(
            insn.mnemonic.as_str(),
            "packed-switch" | "sparse-switch" | "fill-array-data"
        ) {
            for tok in insn.operands.split(|c: char| c == ',' || c.is_whitespace()) {
                let tok = tok.trim().trim_end_matches(',');
                if tok.starts_with(':') {
                    map.insert(tok.to_string(), unit);
                }
            }
        }
        let units = if insn.mnemonic.starts_with(".hex") {
            hex_decode(&insn.operands)
                .map(|b| (b.len() as u32) / 2)
                .unwrap_or(0)
        } else if matches!(
            insn.mnemonic.as_str(),
            ".array-data" | ".packed-switch" | ".sparse-switch"
        ) {
            payload_size_units(&insn.mnemonic, &insn.operands).unwrap_or(0)
        } else if let Some(op) = opcode_for_mnemonic(&insn.mnemonic) {
            format_length(get_opcode_entry(op).format) / 2
        } else {
            0
        };
        let _ = labels;
        unit += units;
    }
    map
}

fn hex_decode(s: &str) -> Result<Vec<u8>> {
    let s: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if s.len() % 2 != 0 {
        return Err(DexError::Txt(format!("odd hex length: {s}")));
    }
    (0..s.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&s[i..i + 2], 16)
                .map_err(|e| DexError::Txt(format!("bad hex: {e}")))
        })
        .collect()
}

fn branch_rel_for(
    insn: &DexTxtInsn,
    here_units: u32,
    labels: &HashMap<String, u32>,
) -> Option<i32> {
    // Find a :label token in operands
    for tok in insn.operands.split(|c: char| c == ',' || c.is_whitespace()) {
        let tok = tok.trim().trim_end_matches(',');
        if tok.starts_with(':') {
            if let Some(&target) = labels.get(tok) {
                return Some(target as i32 - here_units as i32);
            }
            // Also try without ensuring leading colon variants
            let with_colon = if tok.starts_with(':') {
                tok.to_string()
            } else {
                format!(":{tok}")
            };
            if let Some(&target) = labels.get(&with_colon) {
                return Some(target as i32 - here_units as i32);
            }
        }
    }
    None
}

fn build_tries(
    catches: &[DexTxtCatch],
    labels: &HashMap<String, u32>,
    maps: &PoolMaps,
) -> Result<Vec<BuiltTry>> {
    // Group by start/end range
    let mut by_range: HashMap<(u32, u32), BuiltTry> = HashMap::new();
    for c in catches {
        let start = *labels
            .get(&c.start_label)
            .ok_or_else(|| DexError::Txt(format!("unknown catch start {}", c.start_label)))?;
        let end = *labels
            .get(&c.end_label)
            .ok_or_else(|| DexError::Txt(format!("unknown catch end {}", c.end_label)))?;
        let handler = *labels
            .get(&c.handler_label)
            .ok_or_else(|| DexError::Txt(format!("unknown catch handler {}", c.handler_label)))?;
        let count = end.saturating_sub(start) as u16;
        let entry = by_range.entry((start, count as u32)).or_insert(BuiltTry {
            start_unit: start,
            insn_count: count,
            handlers: vec![],
            catch_all: None,
        });
        match &c.exception_type {
            None => entry.catch_all = Some(handler),
            Some(ty) => {
                let idx = maps
                    .type_idx
                    .get(ty)
                    .copied()
                    .ok_or_else(|| DexError::Txt(format!("catch type not in pool: {ty}")))?;
                entry.handlers.push((idx, handler));
            }
        }
    }
    Ok(by_range.into_values().collect())
}

fn estimate_outs(insns: &[DexTxtInsn]) -> u16 {
    let mut max_out = 0u16;
    for insn in insns {
        if insn.mnemonic.starts_with("invoke") {
            // count registers before method ref
            let regs = count_invoke_args(&insn.operands);
            max_out = max_out.max(regs);
        }
    }
    max_out
}

fn count_invoke_args(operands: &str) -> u16 {
    if let Some((regs, _)) = operands.rsplit_once(',') {
        if let Some((a, b)) = regs.split_once("...") {
            let sa = a.trim().trim_start_matches('v').parse::<u16>().unwrap_or(0);
            let sb = b.trim().trim_start_matches('v').parse::<u16>().unwrap_or(0);
            return sb.saturating_sub(sa).saturating_add(1);
        }
        return regs.split(',').filter(|t| t.trim().starts_with('v') || t.trim().starts_with('p')).count() as u16;
    }
    0
}

fn build_debug_ops(method: &DexTxtMethod, maps: &PoolMaps) -> Vec<u8> {
    if method.debug.is_empty() && method.params.is_empty() {
        return Vec::new();
    }
    let line_start = method
        .debug
        .iter()
        .find_map(|d| match d {
            DexTxtDebug::Line(n) => Some(*n),
            _ => None,
        })
        .unwrap_or(1);
    let param_idxs: Vec<Option<u32>> = method
        .params
        .iter()
        .map(|p| maps.string_idx.get(p).copied())
        .collect();
    let mut ops = Vec::new();
    for d in &method.debug {
        match d {
            DexTxtDebug::Line(n) => ops.push(DebugBuilderOp::Line(*n)),
            DexTxtDebug::Local { reg, name, typ } => {
                let reg_n = reg
                    .trim_start_matches('v')
                    .trim_start_matches('p')
                    .parse::<u32>()
                    .unwrap_or(0);
                let Some(name_idx) = maps.string_idx.get(name).copied() else {
                    continue;
                };
                let Some(type_idx) = maps.type_idx.get(typ).copied() else {
                    continue;
                };
                ops.push(DebugBuilderOp::StartLocal {
                    reg: reg_n,
                    name_idx,
                    type_idx,
                });
            }
            DexTxtDebug::Prologue => ops.push(DebugBuilderOp::Prologue),
            DexTxtDebug::Epilogue => ops.push(DebugBuilderOp::Epilogue),
        }
    }
    build_debug_info(line_start, &param_idxs, &ops)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assemble_minimal_class_from_txt() {
        let txt = r#"
.class public LHello;
.super Ljava/lang/Object;

.method public constructor <init>()V
    .registers 1
    return-void
.end method
"#;
        let class = parse_class_file(txt).unwrap();
        let bytes = assemble_classes(&[class]).unwrap();
        let dex = dex_parser::DexFile::parse(&bytes).unwrap();
        assert_eq!(dex.header.class_defs_size, 1);
    }
}
