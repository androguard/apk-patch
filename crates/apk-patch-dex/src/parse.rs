//! Parse dex-txt class files.

use std::path::Path;

use crate::DexError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DexTxtClass {
    pub source_dex: Option<String>,
    pub class_descriptor: String,
    pub access: String,
    pub super_class: Option<String>,
    pub interfaces: Vec<String>,
    pub source_file: Option<String>,
    pub fields: Vec<DexTxtField>,
    pub methods: Vec<DexTxtMethod>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DexTxtField {
    pub access: String,
    pub name: String,
    pub typ: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DexTxtMethod {
    pub access: String,
    pub name: String,
    pub proto: String,
    pub registers: Option<u16>,
    pub insns_hex: Vec<(u32, Vec<u8>)>,
}

pub fn parse_class_file(content: &str) -> Result<DexTxtClass, DexError> {
    let mut class = DexTxtClass {
        source_dex: None,
        class_descriptor: String::new(),
        access: String::new(),
        super_class: None,
        interfaces: Vec::new(),
        source_file: None,
        fields: Vec::new(),
        methods: Vec::new(),
    };

    let mut current_method: Option<DexTxtMethod> = None;
    let mut in_code = false;

    for line in content.lines() {
        let line = line.trim_end();
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            if trimmed.starts_with("# source:") {
                class.source_dex = Some(trimmed["# source:".len()..].trim().into());
            } else if trimmed.starts_with("# class:") {
                class.class_descriptor = trimmed["# class:".len()..].trim().into();
            }
            continue;
        }

        if trimmed == ".end method" {
            if let Some(method) = current_method.take() {
                class.methods.push(method);
            }
            in_code = false;
            continue;
        }
        if trimmed == ".end field" {
            continue;
        }
        if trimmed == ".end code" {
            in_code = false;
            continue;
        }

        if trimmed.starts_with(".class ") {
            let rest = trimmed[".class ".len()..].trim();
            let (access, desc) = split_access_and_name(rest);
            class.access = access;
            class.class_descriptor = desc;
            continue;
        }
        if trimmed.starts_with(".super ") {
            class.super_class = Some(trimmed[".super ".len()..].trim().into());
            continue;
        }
        if trimmed.starts_with(".implements ") {
            class.interfaces.push(trimmed[".implements ".len()..].trim().into());
            continue;
        }
        if trimmed.starts_with(".source ") {
            class.source_file = Some(trimmed[".source ".len()..].trim().into());
            continue;
        }
        if trimmed.starts_with(".field ") {
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
                });
            }
            continue;
        }
        if trimmed.starts_with(".method ") {
            if let Some(method) = current_method.take() {
                class.methods.push(method);
            }
            let rest = trimmed[".method ".len()..].trim();
            if let Some(paren) = rest.find('(') {
                let before = rest[..paren].trim();
                let proto = &rest[paren..];
                let (access, name) = if let Some(sp) = before.rfind(' ') {
                    (before[..sp].trim(), before[sp + 1..].trim())
                } else {
                    ("", before)
                };
                current_method = Some(DexTxtMethod {
                    access: access.into(),
                    name: name.into(),
                    proto: proto.into(),
                    registers: None,
                    insns_hex: Vec::new(),
                });
            }
            continue;
        }
        if trimmed.starts_with(".registers ") {
            if let Some(method) = current_method.as_mut() {
                method.registers =
                    trimmed[".registers ".len()..].trim().parse().ok();
            }
            continue;
        }
        if trimmed == ".code" {
            in_code = true;
            continue;
        }

        if in_code {
            if let Some(method) = current_method.as_mut() {
                if let Some((offset, bytes)) = parse_insn_line(trimmed)? {
                    method.insns_hex.push((offset, bytes));
                }
            }
        }
    }

    if let Some(method) = current_method.take() {
        class.methods.push(method);
    }

    if class.class_descriptor.is_empty() {
        return Err(DexError::Txt("missing .class directive".into()));
    }
    Ok(class)
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

fn parse_insn_line(line: &str) -> Result<Option<(u32, Vec<u8>)>, DexError> {
    let line = line.split('#').next().unwrap_or(line).trim();
    // Instruction lines are `OOOOOOOO: <hex> …`. Skip annotation / continuation
    // lines (e.g. multi-line const-string text containing `https://…`).
    let Some((offset_part, rest)) = split_insn_offset(line) else {
        return Ok(None);
    };
    let offset = u32::from_str_radix(offset_part, 16)
        .map_err(|_| DexError::Txt(format!("invalid offset: {offset_part}")))?;

    let hex_part = rest
        .split_whitespace()
        .next()
        .ok_or_else(|| DexError::Txt(format!("missing hex on line: {line}")))?;
    let bytes = parse_hex_bytes(hex_part)?;
    Ok(Some((offset, bytes)))
}

/// Split `00000010: 1a00…` into (`00000010`, rest). Returns None for non-insn lines.
fn split_insn_offset(line: &str) -> Option<(&str, &str)> {
    let colon = line.find(':')?;
    let offset_part = line[..colon].trim();
    if offset_part.is_empty()
        || !offset_part
            .chars()
            .all(|c| c.is_ascii_hexdigit())
    {
        return None;
    }
    Some((offset_part, &line[colon + 1..]))
}

pub fn parse_hex_bytes(hex: &str) -> Result<Vec<u8>, DexError> {
    let clean: String = hex.chars().filter(|c| !c.is_whitespace()).collect();
    if clean.len() % 2 != 0 {
        return Err(DexError::Txt(format!("odd hex length: {hex}")));
    }
    (0..clean.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&clean[i..i + 2], 16)
                .map_err(|_| DexError::Txt(format!("invalid hex: {}", &clean[i..i + 2])))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_static_method() {
        let txt = r#"
.class public Lcom/example/Foo;
.method public static foo(II)I
    .registers 3
    .code
    00000000: 0e00 return-void
    .end code
.end method
"#;
        let class = parse_class_file(txt).unwrap();
        assert_eq!(class.methods[0].name, "foo");
        assert_eq!(class.methods[0].access, "public static");
        assert_eq!(class.methods[0].proto, "(II)I");
    }

    #[test]
    fn skips_multiline_string_continuation() {
        let txt = r#"
.class public Lcom/example/Foo;
.method static <clinit>()V
    .registers 1
    .code
    00000000: 1a000000      const-string v0, hello
since updates. See https://example.com/issues/1 for details.
    00000004: 0e00          return-void
    .end code
.end method
"#;
        let class = parse_class_file(txt).unwrap();
        let insns = &class.methods[0].insns_hex;
        assert_eq!(insns.len(), 2);
        assert_eq!(insns[0].0, 0);
        assert_eq!(insns[1].0, 4);
    }

    #[test]
    fn parse_simple_class() {
        let txt = r#"
# dex-txt
# source: classes.dex
# class: Lcom/example/Foo;

.class public Lcom/example/Foo;
.super Ljava/lang/Object;

.method public <init>()V
    .registers 1
    .code
    00000000: 0e00 return-void
    .end code
.end method
"#;
        let class = parse_class_file(txt).unwrap();
        assert_eq!(class.class_descriptor, "Lcom/example/Foo;");
        assert_eq!(class.methods.len(), 1);
        assert_eq!(class.methods[0].insns_hex[0].1, vec![0x0e, 0x00]);
    }
}
