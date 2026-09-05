//! Parse dex-txt class files (mnemonic-first AST).

use std::path::Path;

use crate::access::parse_access;
use crate::verify::locals_to_registers;
use crate::DexError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DexTxtClass {
    pub source_dex: Option<String>,
    pub class_descriptor: String,
    pub access: String,
    pub super_class: Option<String>,
    pub interfaces: Vec<String>,
    pub source_file: Option<String>,
    pub annotations: Vec<DexTxtAnnotation>,
    pub fields: Vec<DexTxtField>,
    pub methods: Vec<DexTxtMethod>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DexTxtAnnotation {
    pub visibility: String,
    pub typ: String,
    pub elements: Vec<(String, String)>, // name = literal
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DexTxtField {
    pub access: String,
    pub name: String,
    pub typ: String,
    pub value: Option<String>,
    pub annotations: Vec<DexTxtAnnotation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DexTxtMethod {
    pub access: String,
    pub name: String,
    pub proto: String,
    pub registers: Option<u16>,
    /// Mnemonic-first instructions (preferred).
    pub insns: Vec<DexTxtInsn>,
    /// Legacy hex patches `(byte_offset, bytes)` for migration / bridge.
    pub insns_hex: Vec<(u32, Vec<u8>)>,
    pub catches: Vec<DexTxtCatch>,
    pub debug: Vec<DexTxtDebug>,
    pub params: Vec<String>,
    pub annotations: Vec<DexTxtAnnotation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DexTxtInsn {
    /// Label defined at this instruction (without leading `:` stored with `:`).
    pub label: Option<String>,
    pub mnemonic: String,
    /// Raw operand text after the mnemonic (may be empty).
    pub operands: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DexTxtCatch {
    /// `None` = catchall.
    pub exception_type: Option<String>,
    pub start_label: String,
    pub end_label: String,
    pub handler_label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DexTxtDebug {
    Line(u32),
    Local {
        reg: String,
        name: String,
        typ: String,
    },
    Prologue,
    Epilogue,
}

pub fn parse_class_file(content: &str) -> Result<DexTxtClass, DexError> {
    let mut class = DexTxtClass {
        source_dex: None,
        class_descriptor: String::new(),
        access: String::new(),
        super_class: None,
        interfaces: Vec::new(),
        source_file: None,
        annotations: Vec::new(),
        fields: Vec::new(),
        methods: Vec::new(),
    };

    let mut current_method: Option<DexTxtMethod> = None;
    let mut in_code = false;
    let mut pending_label: Option<String> = None;
    let mut current_field_idx: Option<usize> = None;
    let mut pending_annotations: Vec<DexTxtAnnotation> = Vec::new();

    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0usize;
    while i < lines.len() {
        let line = lines[i].trim_end();
        let trimmed = line.trim();
        i += 1;
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('#') {
            if trimmed.starts_with("# source:") {
                class.source_dex = Some(trimmed["# source:".len()..].trim().into());
            } else if trimmed.starts_with("# class:") {
                class.class_descriptor = trimmed["# class:".len()..].trim().into();
            }
            continue;
        }

        if trimmed == ".end method" {
            if let Some(method) = current_method.as_mut() {
                if let Some(lab) = pending_label.take() {
                    method.insns.push(DexTxtInsn {
                        label: Some(lab),
                        mnemonic: ".mark".into(),
                        operands: String::new(),
                    });
                }
            }
            if let Some(method) = current_method.take() {
                class.methods.push(method);
            }
            in_code = false;
            pending_label = None;
            continue;
        }
        if trimmed == ".end field" {
            current_field_idx = None;
            continue;
        }
        if trimmed == ".end code" {
            in_code = false;
            continue;
        }

        if trimmed.starts_with(".annotation ") {
            let (ann, consumed) = parse_annotation_block(trimmed, &lines[i..])?;
            i += consumed;
            // Attach later: stash until we know target, or attach to open field/method/class
            if let Some(method) = current_method.as_mut() {
                method.annotations.push(ann);
            } else if let Some(idx) = current_field_idx {
                class.fields[idx].annotations.push(ann);
            } else {
                // Before next field/method — hold as pending class-level, or flush to class
                pending_annotations.push(ann);
            }
            continue;
        }

        if trimmed.starts_with(".class ") {
            flush_pending_class_anns(&mut class, &mut pending_annotations);
            let rest = trimmed[".class ".len()..].trim();
            let (access, desc) = split_access_and_name(rest);
            class.access = access;
            class.class_descriptor = desc;
            continue;
        }
        if trimmed.starts_with(".super ") {
            flush_pending_class_anns(&mut class, &mut pending_annotations);
            class.super_class = Some(trimmed[".super ".len()..].trim().into());
            continue;
        }
        if trimmed.starts_with(".implements ") {
            flush_pending_class_anns(&mut class, &mut pending_annotations);
            class
                .interfaces
                .push(trimmed[".implements ".len()..].trim().into());
            continue;
        }
        if trimmed.starts_with(".source ") {
            flush_pending_class_anns(&mut class, &mut pending_annotations);
            class.source_file = Some(unquote(trimmed[".source ".len()..].trim()));
            continue;
        }
        if trimmed.starts_with(".field ") {
            // Class-level annotations appear before fields; field anns are inside `.field`…`.end field`.
            flush_pending_class_anns(&mut class, &mut pending_annotations);
            let rest = trimmed[".field ".len()..].trim();
            if let Some(colon) = rest.rfind(':') {
                let before = rest[..colon].trim();
                let typ = rest[colon + 1..].trim();
                let (access, name) = if let Some(sp) = before.rfind(' ') {
                    (before[..sp].trim(), before[sp + 1..].trim())
                } else {
                    ("", before)
                };
                class.fields.push(DexTxtField {
                    access: access.into(),
                    name: name.into(),
                    typ: typ.into(),
                    value: None,
                    annotations: Vec::new(),
                });
                current_field_idx = Some(class.fields.len() - 1);
            }
            continue;
        }
        if trimmed.starts_with(".value ") {
            if let Some(idx) = current_field_idx {
                class.fields[idx].value = Some(trimmed[".value ".len()..].trim().into());
            }
            continue;
        }
        if trimmed.starts_with(".method ") {
            if let Some(method) = current_method.take() {
                class.methods.push(method);
            }
            current_field_idx = None;
            in_code = true;
            let rest = trimmed[".method ".len()..].trim();
            if let Some(paren) = rest.find('(') {
                let before = rest[..paren].trim();
                let proto = &rest[paren..];
                let (access, name) = if let Some(sp) = before.rfind(' ') {
                    (before[..sp].trim(), before[sp + 1..].trim())
                } else {
                    ("", before)
                };
                let mut anns = Vec::new();
                anns.append(&mut pending_annotations);
                current_method = Some(DexTxtMethod {
                    access: access.into(),
                    name: name.into(),
                    proto: proto.into(),
                    registers: None,
                    insns: Vec::new(),
                    insns_hex: Vec::new(),
                    catches: Vec::new(),
                    debug: Vec::new(),
                    params: Vec::new(),
                    annotations: anns,
                });
            }
            continue;
        }
        if trimmed.starts_with(".registers ") {
            if let Some(method) = current_method.as_mut() {
                method.registers = trimmed[".registers ".len()..].trim().parse().ok();
            }
            in_code = true;
            continue;
        }
        if trimmed.starts_with(".locals ") {
            if let Some(method) = current_method.as_mut() {
                if let Ok(locals) = trimmed[".locals ".len()..].trim().parse::<u16>() {
                    let af = parse_access(&method.access);
                    method.registers = Some(locals_to_registers(locals, &method.proto, af));
                }
            }
            in_code = true;
            continue;
        }
        if trimmed.starts_with(".param ") {
            if let Some(method) = current_method.as_mut() {
                method
                    .params
                    .push(unquote(trimmed[".param ".len()..].trim()));
            }
            continue;
        }
        if trimmed.starts_with(".line ") {
            if let Some(method) = current_method.as_mut() {
                if let Ok(n) = trimmed[".line ".len()..].trim().parse() {
                    method.debug.push(DexTxtDebug::Line(n));
                }
            }
            continue;
        }
        if trimmed.starts_with(".local ") {
            if let Some(method) = current_method.as_mut() {
                let rest = trimmed[".local ".len()..].trim();
                let parts: Vec<&str> = rest.split_whitespace().collect();
                if parts.len() >= 3 {
                    method.debug.push(DexTxtDebug::Local {
                        reg: parts[0].into(),
                        name: unquote(parts[1]),
                        typ: parts[2].into(),
                    });
                }
            }
            continue;
        }
        if trimmed == ".prologue" {
            if let Some(method) = current_method.as_mut() {
                method.debug.push(DexTxtDebug::Prologue);
            }
            continue;
        }
        if trimmed == ".epilogue" {
            if let Some(method) = current_method.as_mut() {
                method.debug.push(DexTxtDebug::Epilogue);
            }
            continue;
        }
        if trimmed.starts_with(".catch ") || trimmed.starts_with(".catchall") {
            if let Some(method) = current_method.as_mut() {
                if let Some(lab) = pending_label.take() {
                    method.insns.push(DexTxtInsn {
                        label: Some(lab),
                        mnemonic: ".mark".into(),
                        operands: String::new(),
                    });
                }
                if let Some(c) = parse_catch_line(trimmed)? {
                    method.catches.push(c);
                }
            }
            continue;
        }
        if trimmed == ".code" {
            in_code = true;
            continue;
        }

        // Payload blocks
        if trimmed.starts_with(".array-data")
            || trimmed.starts_with(".packed-switch")
            || trimmed.starts_with(".sparse-switch")
        {
            if let Some(method) = current_method.as_mut() {
                let (insn, consumed) =
                    parse_payload_block(trimmed, &lines[i..], pending_label.take())?;
                i += consumed;
                method.insns.push(insn);
            }
            continue;
        }

        // Standalone label line: `:L_00000010`
        if trimmed.starts_with(':') && !trimmed.contains(' ') && !trimmed.ends_with(':') {
            if !trimmed[1..].contains(':') {
                if let (Some(method), Some(lab)) =
                    (current_method.as_mut(), pending_label.take())
                {
                    method.insns.push(DexTxtInsn {
                        label: Some(lab),
                        mnemonic: ".mark".into(),
                        operands: String::new(),
                    });
                }
                pending_label = Some(trimmed.to_string());
                continue;
            }
        }
        // Label with trailing colon only: `:Lfoo:`
        if trimmed.starts_with(':') && trimmed.ends_with(':') && trimmed.matches(':').count() == 2 {
            if let (Some(method), Some(lab)) = (current_method.as_mut(), pending_label.take()) {
                method.insns.push(DexTxtInsn {
                    label: Some(lab),
                    mnemonic: ".mark".into(),
                    operands: String::new(),
                });
            }
            pending_label = Some(trimmed.trim_end_matches(':').to_string());
            continue;
        }

        if in_code {
            if let Some(method) = current_method.as_mut() {
                if let Some((offset, bytes)) = parse_legacy_hex_insn(trimmed)? {
                    method.insns_hex.push((offset, bytes.clone()));
                    if let Some(insn) = parse_mnemonic_from_legacy(trimmed, pending_label.take())? {
                        method.insns.push(insn);
                    } else {
                        method.insns.push(DexTxtInsn {
                            label: pending_label.take(),
                            mnemonic: format!(".hex@{offset:08x}"),
                            operands: hex_encode(&bytes),
                        });
                    }
                    continue;
                }
                if let Some(insn) = parse_mnemonic_insn(trimmed, pending_label.take())? {
                    method.insns.push(insn);
                }
            }
        } else {
            // Class-level annotations after header may still be pending
            flush_pending_class_anns(&mut class, &mut pending_annotations);
        }
    }

    flush_pending_class_anns(&mut class, &mut pending_annotations);
    if let Some(method) = current_method.take() {
        class.methods.push(method);
    }

    if class.class_descriptor.is_empty() {
        return Err(DexError::Txt("missing .class directive".into()));
    }
    Ok(class)
}

fn flush_pending_class_anns(class: &mut DexTxtClass, pending: &mut Vec<DexTxtAnnotation>) {
    class.annotations.append(pending);
}

fn parse_annotation_block(
    first: &str,
    rest: &[&str],
) -> Result<(DexTxtAnnotation, usize), DexError> {
    // `.annotation runtime Landroid/…;`
    let header = first[".annotation ".len()..].trim();
    let (visibility, typ) = if let Some(sp) = header.find(' ') {
        (header[..sp].trim().to_string(), header[sp + 1..].trim().to_string())
    } else {
        return Err(DexError::Txt(format!("bad .annotation header: {first}")));
    };
    let mut elements = Vec::new();
    let mut consumed = 0usize;
    let mut i = 0usize;
    while i < rest.len() {
        let line = rest[i];
        consumed += 1;
        i += 1;
        let t = line.trim();
        if t == ".end annotation" {
            return Ok((
                DexTxtAnnotation {
                    visibility,
                    typ,
                    elements,
                },
                consumed,
            ));
        }
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if let Some((name, val)) = t.split_once('=') {
            let name = name.trim().to_string();
            let val = val.trim();
            if val.starts_with(".subannotation ") {
                let mut block = val.to_string();
                block.push('\n');
                let mut depth = 1i32;
                while i < rest.len() {
                    let l = rest[i];
                    consumed += 1;
                    i += 1;
                    block.push_str(l);
                    block.push('\n');
                    let lt = l.trim();
                    if lt.starts_with(".subannotation ") {
                        depth += 1;
                    } else if lt == ".end subannotation" {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                }
                if depth != 0 {
                    return Err(DexError::Txt("unclosed .subannotation".into()));
                }
                elements.push((name, block));
            } else {
                elements.push((name, val.to_string()));
            }
        }
    }
    Err(DexError::Txt("unclosed .annotation".into()))
}

fn parse_payload_block(
    first: &str,
    rest: &[&str],
    label: Option<String>,
) -> Result<(DexTxtInsn, usize), DexError> {
    let (mnemonic, end_marker, header_ops) = if first.starts_with(".array-data") {
        (
            ".array-data",
            ".end array-data",
            first[".array-data".len()..].trim().to_string(),
        )
    } else if first.starts_with(".packed-switch") {
        (
            ".packed-switch",
            ".end packed-switch",
            first[".packed-switch".len()..].trim().to_string(),
        )
    } else if first.starts_with(".sparse-switch") {
        (
            ".sparse-switch",
            ".end sparse-switch",
            first[".sparse-switch".len()..].trim().to_string(),
        )
    } else {
        return Err(DexError::Txt(format!("unknown payload: {first}")));
    };
    let mut body = Vec::new();
    if !header_ops.is_empty() {
        body.push(header_ops);
    }
    let mut consumed = 0usize;
    for line in rest {
        consumed += 1;
        let t = line.trim();
        if t == end_marker {
            return Ok((
                DexTxtInsn {
                    label,
                    mnemonic: mnemonic.into(),
                    operands: body.join("\n"),
                },
                consumed,
            ));
        }
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        body.push(t.to_string());
    }
    Err(DexError::Txt(format!("unclosed {mnemonic}")))
}

pub fn parse_class_path(path: &Path) -> Result<DexTxtClass, DexError> {
    let content = std::fs::read_to_string(path)?;
    parse_class_file(&content)
}

fn split_access_and_name(rest: &str) -> (String, String) {
    let rest = rest.trim();
    if let Some(idx) = rest.rfind(' ') {
        (rest[..idx].trim().into(), rest[idx + 1..].trim().into())
    } else {
        (String::new(), rest.into())
    }
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        s[1..s.len() - 1].into()
    } else {
        s.into()
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn parse_catch_line(trimmed: &str) -> Result<Option<DexTxtCatch>, DexError> {
    let catchall = trimmed.starts_with(".catchall");
    let rest = if catchall {
        trimmed[".catchall".len()..].trim()
    } else {
        trimmed[".catch ".len()..].trim()
    };
    let exception_type = if catchall {
        None
    } else {
        let brace = rest
            .find('{')
            .ok_or_else(|| DexError::Txt(format!("bad .catch: {trimmed}")))?;
        Some(rest[..brace].trim().to_string())
    };
    let body = rest;
    let open = body
        .find('{')
        .ok_or_else(|| DexError::Txt(format!("bad catch braces: {trimmed}")))?;
    let close = body
        .find('}')
        .ok_or_else(|| DexError::Txt(format!("bad catch braces: {trimmed}")))?;
    let range = body[open + 1..close].trim();
    let handler = body[close + 1..].trim().to_string();
    let (start_label, end_label) = if let Some((a, b)) = range.split_once("..") {
        (a.trim().to_string(), b.trim().to_string())
    } else {
        return Err(DexError::Txt(format!("bad catch range: {trimmed}")));
    };
    Ok(Some(DexTxtCatch {
        exception_type,
        start_label,
        end_label,
        handler_label: handler,
    }))
}

fn parse_legacy_hex_insn(trimmed: &str) -> Result<Option<(u32, Vec<u8>)>, DexError> {
    // `0000: 0e00` or `0000: 0e00  # return-void` or `0000: 0e00          return-void`
    let Some((off_s, rest)) = trimmed.split_once(':') else {
        return Ok(None);
    };
    let off_s = off_s.trim();
    if off_s.is_empty() || !off_s.chars().all(|c| c.is_ascii_hexdigit()) {
        return Ok(None);
    }
    let Ok(offset) = u32::from_str_radix(off_s, 16) else {
        return Ok(None);
    };
    let rest = rest.split('#').next().unwrap_or(rest).trim();
    // Only consume leading hex byte pairs; stop at mnemonic text.
    let mut hex = String::new();
    for tok in rest.split_whitespace() {
        if tok.chars().all(|c| c.is_ascii_hexdigit()) && tok.len() % 2 == 0 {
            hex.push_str(tok);
        } else {
            break;
        }
    }
    if hex.len() < 2 {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    for i in (0..hex.len()).step_by(2) {
        bytes.push(
            u8::from_str_radix(&hex[i..i + 2], 16)
                .map_err(|e| DexError::Txt(format!("bad hex: {e}")))?,
        );
    }
    Ok(Some((offset, bytes)))
}

fn parse_mnemonic_from_legacy(
    trimmed: &str,
    label: Option<String>,
) -> Result<Option<DexTxtInsn>, DexError> {
    // Prefer explicit `# mnemonic` commentary.
    if let Some((_, after_hash)) = trimmed.split_once('#') {
        let text = after_hash.trim();
        if !text.is_empty() {
            return parse_mnemonic_insn(text, label);
        }
    }
    // Or trailing mnemonic after hex tokens: `0000: 0e00    return-void`
    let Some((_, rest)) = trimmed.split_once(':') else {
        return Ok(None);
    };
    let rest = rest.trim();
    let mut saw_hex = false;
    for tok in rest.split_whitespace() {
        if tok.chars().all(|c| c.is_ascii_hexdigit()) && tok.len() % 2 == 0 {
            saw_hex = true;
            continue;
        }
        if saw_hex {
            let rem = rest[rest.find(tok).unwrap_or(0)..].trim();
            return parse_mnemonic_insn(rem, label);
        }
        break;
    }
    Ok(None)
}

fn parse_mnemonic_insn(trimmed: &str, label: Option<String>) -> Result<Option<DexTxtInsn>, DexError> {
    let trimmed = trimmed.trim();
    // Trailing ` # hex` commentary must not become operands — but `#` inside
    // string literals (e.g. "Resource ID #0x7f") is content.
    let trimmed = strip_trailing_hash_comment(trimmed);
    if trimmed.is_empty() || (trimmed.starts_with('.') && !trimmed.starts_with(".hex")) {
        if trimmed.starts_with(".hex ") {
            return Ok(Some(DexTxtInsn {
                label,
                mnemonic: ".hex".into(),
                operands: trimmed[".hex ".len()..].trim().into(),
            }));
        }
        return Ok(None);
    }
    if trimmed.starts_with(".hex ") {
        return Ok(Some(DexTxtInsn {
            label,
            mnemonic: ".hex".into(),
            operands: trimmed[".hex ".len()..].trim().into(),
        }));
    }
    let (mnemonic, operands) = if let Some((m, rest)) = trimmed.split_once(char::is_whitespace) {
        (m.to_string(), rest.trim().to_string())
    } else {
        (trimmed.to_string(), String::new())
    };
    Ok(Some(DexTxtInsn {
        label,
        mnemonic,
        operands,
    }))
}

fn strip_trailing_hash_comment(s: &str) -> &str {
    let bytes = s.as_bytes();
    let mut in_str = false;
    let mut escape = false;
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if in_str {
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if b == b'"' {
            in_str = true;
            i += 1;
            continue;
        }
        if b == b'#' && (i == 0 || bytes[i - 1] == b' ') {
            return s[..i].trim_end();
        }
        i += 1;
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mnemonic_first() {
        let txt = r#"
.class public LHello;
.super Ljava/lang/Object;

.method public constructor <init>()V
    .registers 1
    return-void
.end method
"#;
        let c = parse_class_file(txt).unwrap();
        assert_eq!(c.class_descriptor, "LHello;");
        assert_eq!(c.methods[0].insns[0].mnemonic, "return-void");
    }

    #[test]
    fn parse_legacy_hex_still_works() {
        let txt = r#"
.class public LHello;
.super Ljava/lang/Object;

.method public constructor <init>()V
    .registers 1
    0000: 0e00  # return-void
.end method
"#;
        let c = parse_class_file(txt).unwrap();
        assert_eq!(c.methods[0].insns[0].mnemonic, "return-void");
    }

    #[test]
    fn parse_catch() {
        let txt = r#"
.class public LHello;
.super Ljava/lang/Object;

.method public foo()V
    .registers 2
    :Lstart
    const/4 v0, 0
    :Lend
    return-void
    :Lhandler
    move-exception v0
    return-void
    .catch Ljava/lang/Exception; { :Lstart .. :Lend } :Lhandler
.end method
"#;
        let c = parse_class_file(txt).unwrap();
        assert_eq!(c.methods[0].catches.len(), 1);
    }

    #[test]
    fn parse_locals_annotation_payload() {
        let txt = r#"
.class public LHello;
.super Ljava/lang/Object;

.annotation runtime LFoo;
    value = "x"
.end annotation

.field public static final S:Ljava/lang/String;
    .value "hi"
.end field

.method public static bar(I)V
    .locals 1
    :Ldata
    .array-data 4
        0x1
        0x2
    .end array-data
    return-void
.end method
"#;
        let c = parse_class_file(txt).unwrap();
        assert_eq!(c.annotations.len(), 1);
        assert_eq!(c.fields[0].value.as_deref(), Some("\"hi\""));
        // static bar(I)V → 1 param + locals 1 = 2 registers
        assert_eq!(c.methods[0].registers, Some(2));
        assert_eq!(c.methods[0].insns[0].mnemonic, ".array-data");
    }
}
