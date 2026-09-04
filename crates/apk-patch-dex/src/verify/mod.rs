//! Method verification: structural checks + baksmali-style register-type dataflow.

mod analyzer;
mod regtype;

pub use analyzer::analyze_method;
pub use regtype::{Category, RegType};

use crate::access::proto_ins_words;
use crate::parse::{DexTxtClass, DexTxtInsn, DexTxtMethod};
use crate::DexError;

/// Convert `.locals N` to total registers given method proto and access.
pub fn locals_to_registers(locals: u16, proto: &str, access_flags: u32) -> u16 {
    let is_static = access_flags & 0x0008 != 0;
    let ins = proto_ins_words(proto, is_static);
    locals.saturating_add(ins)
}

/// Convert `.registers N` to `.locals` count.
pub fn registers_to_locals(registers: u16, proto: &str, access_flags: u32) -> u16 {
    let is_static = access_flags & 0x0008 != 0;
    let ins = proto_ins_words(proto, is_static);
    registers.saturating_sub(ins)
}

/// Hard-error verification before assemble (structural + register-type prover).
pub fn verify_class(class: &DexTxtClass) -> Result<(), DexError> {
    for m in &class.methods {
        verify_method(m, &class.class_descriptor)?;
    }
    Ok(())
}

/// Structural verification for one method (labels, register indices, tries).
pub fn verify_method_structural(method: &DexTxtMethod) -> Result<(), DexError> {
    let registers = method.registers.unwrap_or(0);
    let mut labels = std::collections::HashSet::new();
    for insn in &method.insns {
        if let Some(ref lab) = insn.label {
            if !labels.insert(lab.clone()) {
                return Err(DexError::Txt(format!(
                    "duplicate label {lab} in method {}{}",
                    method.name, method.proto
                )));
            }
        }
    }
    for insn in &method.insns {
        check_regs(insn, registers, &method.name)?;
        for tok in label_tokens(insn) {
            if !labels.contains(tok) {
                return Err(DexError::Txt(format!(
                    "unknown label {tok} in {}{} ({})",
                    method.name, method.proto, insn.mnemonic
                )));
            }
        }
    }
    for c in &method.catches {
        for lab in [&c.start_label, &c.end_label, &c.handler_label] {
            if !labels.contains(lab) {
                return Err(DexError::Txt(format!(
                    "catch refers to unknown label {lab} in {}{}",
                    method.name, method.proto
                )));
            }
        }
    }
    Ok(())
}

/// Full verification: structural then register-type dataflow.
pub fn verify_method(method: &DexTxtMethod, class_descriptor: &str) -> Result<(), DexError> {
    verify_method_structural(method)?;
    analyze_method(method, class_descriptor)?;
    Ok(())
}

fn label_tokens(insn: &DexTxtInsn) -> Vec<&str> {
    let mut out = Vec::new();
    let mut in_str = false;
    let bytes = insn.operands.as_bytes();
    let mut start = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'"' {
            in_str = !in_str;
            continue;
        }
        if in_str {
            continue;
        }
        let is_sep = b == b',' || b == b'{' || b == b'}' || b.is_ascii_whitespace();
        if is_sep {
            if start < i {
                let tok = insn.operands[start..i].trim().trim_end_matches(',');
                if tok.starts_with(':') {
                    out.push(tok);
                }
            }
            start = i + 1;
        }
    }
    if start < insn.operands.len() {
        let tok = insn.operands[start..].trim().trim_end_matches(',');
        if tok.starts_with(':') {
            out.push(tok);
        }
    }
    out
}

fn check_regs(insn: &DexTxtInsn, registers: u16, method: &str) -> Result<(), DexError> {
    if registers == 0 {
        return Ok(());
    }
    for tok in insn.operands.split(|c: char| c == ',' || c.is_whitespace()) {
        let tok = tok.trim().trim_end_matches(',');
        if let Some(rest) = tok.strip_prefix('v') {
            if let Ok(n) = rest.parse::<u16>() {
                if n >= registers {
                    return Err(DexError::Txt(format!(
                        "register {tok} out of range (.registers {registers}) in {method}"
                    )));
                }
            }
        }
    }
    Ok(())
}
